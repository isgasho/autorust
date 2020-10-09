#![allow(unused_variables, dead_code)]
use crate::{spec, OperationVerb, Reference, ResolvedSchema, Result, Spec};
use autorust_openapi::{DataType, Operation, Parameter, PathItem, ReferenceOr, Schema};
use heck::{CamelCase, SnakeCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use regex::Regex;
use serde_json::Value;
use std::{collections::HashSet, path::Path};

/// code generation context
pub struct CodeGen {
    pub spec: Spec,
}
impl CodeGen {
    pub fn from_file<P: AsRef<Path>>(input_file: P) -> Result<Self> {
        let spec = Spec::read_file(input_file)?;
        Ok(Self { spec })
    }

    pub fn create_models(&self) -> Result<TokenStream> {
        let mut file = TokenStream::new();
        file.extend(create_generated_by_header());
        file.extend(quote! {
            #![allow(non_camel_case_types)]
            use crate::*;
            use serde::{Deserialize, Serialize};
        });
        let (root_path, root_doc) = self.spec.docs.get_index(0).unwrap();
        let schemas = &self
            .spec
            .resolve_schema_map(root_path, &root_doc.definitions)?;
        for (name, schema) in schemas {
            if is_schema_an_array(schema) {
                file.extend(self.create_vec_alias(root_path, name, schema)?);
            } else {
                for stream in self.create_struct(root_path, name, schema)? {
                    file.extend(stream);
                }
            }
        }
        Ok(file)
    }

    pub fn create_client(&self) -> Result<TokenStream> {
        let mut file = TokenStream::new();
        file.extend(create_generated_by_header());
        file.extend(quote! {
            #![allow(unused_mut)]
            #![allow(unused_variables)]
            use crate::*;
            use anyhow::{Error, Result};
        });
        let param_re = Regex::new(r"\{(\w+)\}").unwrap();
        let (doc_file, doc) = self.spec.root();
        let paths = self.spec.resolve_path_map(doc_file, &doc.paths)?;
        for (path, item) in &paths {
            // println!("{}", path);
            for op in spec::pathitem_operations(item) {
                // println!("{:?}", op.operation_id);
                file.extend(create_function(self, path, item, &op, &param_re))
            }
        }
        Ok(file)
    }

    fn create_vec_alias(
        &self,
        doc_file: &Path,
        alias_name: &str,
        schema: &ResolvedSchema,
    ) -> Result<TokenStream> {
        let items = get_schema_array_items(&schema.schema)?;
        let typ = ident(&alias_name.to_camel_case());
        let items_typ = get_type_for_schema(&items)?;
        Ok(quote! { pub type #typ = Vec<#items_typ>; })
    }

    fn create_struct(
        &self,
        doc_file: &Path,
        struct_name: &str,
        schema: &ResolvedSchema,
    ) -> Result<Vec<TokenStream>> {
        let mut streams = vec![];
        let mut props = TokenStream::new();
        let nm = ident(&struct_name.to_camel_case());
        let required: HashSet<&str> = schema.schema.required.iter().map(String::as_str).collect();

        let properties = self
            .spec
            .resolve_schema_map(doc_file, &schema.schema.properties)?;
        for (property_name, property) in &properties {
            let nm = ident(&property_name.to_snake_case());
            let (field_tp_name, field_tp) =
                self.create_struct_field_type(doc_file, struct_name, property_name, property)?;
            let is_required = required.contains(property_name.as_str());
            let field_tp_name = require(is_required, field_tp_name);

            if let Some(field_tp) = field_tp {
                streams.push(field_tp);
            }
            let skip_serialization_if = if is_required {
                quote! {}
            } else {
                quote! {skip_serializing_if = "Option::is_none"}
            };
            let rename = if &nm.to_string() == property_name {
                if is_required {
                    quote! {}
                } else {
                    quote! {#[serde(#skip_serialization_if)]}
                }
            } else {
                if is_required {
                    quote! {#[serde(rename = #property_name)]}
                } else {
                    quote! {#[serde(rename = #property_name, #skip_serialization_if)]}
                }
            };
            let prop = quote! {
                #rename
                #nm: #field_tp_name,
            };
            props.extend(prop);
        }

        let st = quote! {
            #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
            pub struct #nm {
                #props
            }
        };
        streams.push(TokenStream::from(st));
        Ok(streams)
    }

    /// Creates the type reference for a struct field from a struct property.
    /// Optionally, creates an inner struct for an enum or a private schema.
    fn create_struct_field_type(
        &self,
        doc_file: &Path,
        struct_name: &str,
        property_name: &str,
        property: &ResolvedSchema,
    ) -> Result<(TokenStream, Option<TokenStream>)> {
        match &property.ref_key {
            Some(ref_key) => {
                let tp = ident(&ref_key.name.to_camel_case());
                Ok((tp, None))
            }
            None => {
                let schema_type = property.schema.common.type_.as_ref();
                let enum_values = enum_values_as_strings(&property.schema.common.enum_);
                let mut enum_ts: Option<TokenStream> = None;
                let tp = if enum_values.len() > 0 {
                    enum_ts = Some(create_enum(struct_name, property_name, enum_values));
                    let ns = ident(&struct_name.to_snake_case());
                    let id = ident(&property_name.to_camel_case());
                    TokenStream::from(quote! {#ns::#id})
                } else {
                    let unknown_type = quote!(UnknownType);
                    if let Some(schema_type) = schema_type {
                        let format = property.schema.common.format.as_deref();
                        match schema_type {
                            DataType::Array => {
                                let items = get_schema_array_items(&property.schema)?;
                                let vec_items_typ = get_type_for_schema(&items)?;
                                quote! { Vec<#vec_items_typ> }
                            }
                            DataType::Integer if format == Some("int32") => quote! { i32 },
                            DataType::Integer => quote! { i64 },
                            DataType::Number if format == Some("float") => quote! { f32 },
                            DataType::Number => quote! { f64 },
                            DataType::String => quote! { String },
                            DataType::Boolean => quote! { bool },
                            DataType::Object => quote! { serde_json::Value },
                        }
                    } else {
                        eprintln!(
                            "UnknownType {} {} {}",
                            doc_file.display(),
                            struct_name,
                            property_name
                        );
                        unknown_type
                    }
                };
                Ok((tp, enum_ts))
            }
        }
    }
}

fn is_schema_an_array(schema: &spec::ResolvedSchema) -> bool {
    matches!(&schema.schema.common.type_, Some(DataType::Array))
}

fn get_schema_array_items(schema: &Schema) -> Result<&ReferenceOr<Schema>> {
    Ok(schema
        .common
        .items
        .as_ref()
        .as_ref()
        .ok_or_else(|| format!("array expected to have items"))?)
}

fn create_generated_by_header() -> TokenStream {
    let version = env!("CARGO_PKG_VERSION");
    let comment = format!("generated by AutoRust {}", &version);
    quote! { #![doc = #comment] }
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        // https://doc.rust-lang.org/grammar.html#keywords
        "abstract"
            | "alignof"
            | "as"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "do"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "offsetof"
            | "override"
            | "priv"
            | "proc"
            | "pub"
            | "pure"
            | "ref"
            | "return"
            | "Self"
            | "self"
            | "sizeof"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "typeof"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    )
}

fn create_enum(struct_name: &str, property_name: &str, enum_values: Vec<&str>) -> TokenStream {
    let mut values = TokenStream::new();

    enum_values.iter().for_each(|name| {
        let nm = ident(&name.to_camel_case());
        let rename = if &nm.to_string() == name {
            quote! {}
        } else {
            quote! { #[serde(rename = #name)] }
        };
        let value = quote! {
            #rename
            #nm,
        };
        values.extend(value);
    });

    let ns = ident(&struct_name.to_snake_case());
    let nm = ident(&property_name.to_camel_case());

    let enm = quote! {
        mod #ns {
            use super::*;
            #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
            pub enum #nm {
                #values
            }
        }
    };

    TokenStream::from(enm)
}

/// Wraps a type in an Option if is not required.
fn require(is_required: bool, tp: TokenStream) -> TokenStream {
    if is_required {
        tp
    } else {
        quote! { Option<#tp> }
    }
}

fn ident(text: &str) -> TokenStream {
    let text = text.replace(".", "_");
    // prefix with underscore if starts with invalid character
    let text = match text.chars().next().unwrap() {
        '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '0' => format!("_{}", text),
        _ => text.to_owned(),
    };
    let idt = if is_keyword(&text) {
        format_ident!("{}_", text)
    } else {
        format_ident!("{}", text)
    };
    idt.into_token_stream()
}

fn enum_values_as_strings(values: &Vec<Value>) -> Vec<&str> {
    values
        .iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

/// example: pub type Pets = Vec<Pet>;
fn trim_ref(path: &str) -> String {
    let pos = path.rfind('/').map_or(0, |i| i + 1);
    path[pos..].to_string()
}

// simple types in the url
fn map_type(param_type: &DataType) -> TokenStream {
    match param_type {
        DataType::String => quote! { &str },
        DataType::Integer => quote! { i64 },
        _ => quote! { map_type }, // TODO may be Err instead
    }
}

fn get_param_type(param: &Parameter) -> Result<TokenStream> {
    // let required = required.map_or(false); // TODO
    if let Some(param_type) = &param.common.type_ {
        Ok(map_type(param_type))
    } else if let Some(schema) = &param.schema {
        Ok(get_type_for_schema(schema)?)
    } else {
        let idt = ident("NoParamType1");
        Ok(quote! { #idt }) // TOOD may be Err instead
    }
}

fn get_param_name_and_type(param: &Parameter) -> Result<TokenStream> {
    let name = ident(&param.name.to_snake_case());
    let typ = get_param_type(param)?;
    Ok(quote! { #name: #typ })
}

fn parse_params(param_re: &Regex, path: &str) -> Vec<String> {
    // capture 0 is the whole match and 1 is the actual capture like other languages
    // param_re.find_iter(path).into_iter().map(|m| m.as_str().to_string()).collect()
    param_re
        .captures_iter(path)
        .into_iter()
        .map(|c| c[1].to_string())
        .collect()
}

fn format_path(param_re: &Regex, path: &str) -> String {
    param_re.replace_all(path, "{}").to_string()
}

fn create_function_params(cg: &CodeGen, op: &Operation) -> Result<TokenStream> {
    let doc_file = cg.spec.root_file(); // TODO pass in
    let parameters: Vec<Parameter> = cg.spec.resolve_parameters(doc_file, &op.parameters)?;
    let mut params: Vec<TokenStream> = Vec::new();
    for param in &parameters {
        params.push(get_param_name_and_type(param)?);
    }
    let slf = quote! { configuration: &Configuration };
    params.insert(0, slf);
    Ok(quote! { #(#params),* })
}

fn get_type_for_schema(schema: &ReferenceOr<Schema>) -> Result<TokenStream> {
    match schema {
        ReferenceOr::Reference { reference, .. } => {
            let rf = Reference::parse(&reference)?;
            let idt = ident(
                &rf.name
                    .ok_or_else(|| format!("no name for ref {}", reference))?,
            );
            Ok(quote! { #idt })
        }
        ReferenceOr::Item(schema) => {
            if let Some(schema_type) = &schema.common.type_ {
                let format = schema.common.format.as_deref();
                let ts = match schema_type {
                    DataType::Array => {
                        let items = get_schema_array_items(schema)?;
                        let vec_items_typ = get_type_for_schema(&items)?;
                        quote! {Vec<#vec_items_typ>}
                    }
                    DataType::Integer => {
                        if format == Some("int32") {
                            quote! {i32}
                        } else {
                            quote! {i64}
                        }
                    }
                    DataType::Number => {
                        if format == Some("float") {
                            quote! {f32}
                        } else {
                            quote! {f64}
                        }
                    }
                    DataType::String => quote! {String},
                    DataType::Boolean => quote! {bool},
                    DataType::Object => quote! {serde_json::Value},
                };
                return Ok(ts);
            }

            // TODO probably need to create a struct
            // and have a way to name it
            let idt = ident("NoParamType2");
            Ok(quote! { #idt })
        }
    }
}

fn create_function_return(verb: &OperationVerb) -> Result<TokenStream> {
    // TODO error responses
    // TODO union of responses
    for (_http_code, rsp) in verb.operation().responses.iter() {
        // println!("response key {:#?} {:#?}", key, rsp);
        if let Some(schema) = &rsp.schema {
            let tp = get_type_for_schema(schema)?;
            return Ok(quote! { Result<#tp> });
        }
    }
    Ok(quote! { Result<()> })
}

/// Creating a function name from the path and verb when an operationId is not specified.
/// All azure-rest-api-specs operations should have an operationId.
fn create_function_name(path: &str, verb_name: &str) -> String {
    let mut path = path
        .split('/')
        .filter(|&x| !x.is_empty())
        .collect::<Vec<_>>();
    path.push(verb_name);
    path.join("_")
}

fn create_function(
    cg: &CodeGen,
    path: &str,
    item: &PathItem,
    operation_verb: &OperationVerb,
    param_re: &Regex,
) -> Result<TokenStream> {
    let fname = ident(
        operation_verb
            .operation()
            .operation_id
            .as_ref()
            .unwrap_or(&create_function_name(path, operation_verb.verb_name()))
            .to_snake_case()
            .as_ref(),
    );

    let params = parse_params(param_re, path);
    // println!("path params {:#?}", params);
    let params: Vec<_> = params.iter().map(|s| ident(&s.to_snake_case())).collect();
    let uri_str_args = quote! { #(#params),* };

    let fpath = format!("{{}}{}", &format_path(param_re, path));

    // get path parameters
    // Option if not required
    let fparams = create_function_params(cg, operation_verb.operation())?;

    // see if there is a body parameter
    let fresponse = create_function_return(operation_verb)?;

    let client_verb = match operation_verb {
        OperationVerb::Get(_) => quote! { client.get(uri_str) },
        OperationVerb::Post(_) => quote! { client.post(uri_str) },
        OperationVerb::Put(_) => quote! { client.put(uri_str) },
        OperationVerb::Patch(_) => quote! { client.patch(uri_str) },
        OperationVerb::Delete(_) => quote! { client.delete(uri_str) },
        OperationVerb::Options(_) => quote! { client.options(uri_str) },
        OperationVerb::Head(_) => quote! { client.head(uri_str) },
    };

    // TODO #17 decode the different errors depending on http status
    // TODO #18 other callbacks like auth
    let func = quote! {
        pub async fn #fname(#fparams) -> #fresponse {
            let client = &configuration.client;
            let uri_str = &format!(#fpath, &configuration.base_path, #uri_str_args);
            let mut req_builder = #client_verb;
            let req = req_builder.build()?;
            let res = client.execute(req).await?;
            match res.error_for_status_ref() {
                Ok(_) => Ok(res.json().await?),
                Err(err) => {
                    let e = Error::new(err);
                    let e = e.context(res.text().await?);
                    Err(e)
                },
            }
        }
    };
    Ok(TokenStream::from(func))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ident_odata_next_link() {
        let idt = "odata.nextLink".to_snake_case();
        assert_eq!(idt, "odata.next_link");
        let idt = ident(&idt);
        assert_eq!(idt.to_string(), "odata_next_link");
    }

    #[test]
    fn test_ident_three_dot_two() {
        let idt = ident("3.2");
        assert_eq!(idt.to_string(), "_3_2");
    }

    #[test]
    fn test_create_function_name() {
        assert_eq!(create_function_name("/pets", "get"), "pets_get");
    }
}
