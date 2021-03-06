// cargo run --example gen_mgmt --release
// https://github.com/Azure/azure-rest-api-specs/blob/master/specification/compute/resource-manager
use autorust_codegen::{
    self, cargo_toml,
    config_parser::{to_api_version, to_mod_name},
    get_mgmt_configs, lib_rs, path, Config, PropertyName, SpecConfigs,
};
use heck::SnakeCase;
use snafu::{ResultExt, Snafu};
use std::{collections::HashSet, fs, path::PathBuf};

const OUTPUT_FOLDER: &str = "../azure-sdk-for-rust/services/mgmt";

const ONLY_SERVICES: &[&str] = &[
    // "vmware",
    // "network",
    // "cosmos-db",
];

const SKIP_SERVICES: &[&str] = &[
    "automation",                 // TODO #81 DataType::File
    "deploymentmanager",          // TODO #80 path parameters
    "deviceprovisioningservices", // TODO #82 certificate_name used as parameter more than once
    "dnc",                        // https://github.com/Azure/azure-rest-api-specs/pull/11578 two ControllerDetails types
    "mixedreality",               // TODO #83 AccountKeyRegenerateRequest not generated
    "netapp",                     // Ident "10minutely"
    "powerplatform",              // https://github.com/Azure/azure-rest-api-specs/pull/11580 incorrect ref & duplicate Operations_List
    "service-map",                // Ident "Ref:machine"
    "servicefabric",              // https://github.com/Azure/azure-rest-api-specs/pull/11581 allOf mistakes and duplicate Operations_List
    "servicefabricmanagedclusters",
    "web", // TODO #81 DataType::File
];

const SKIP_SERVICE_TAGS: &[(&str, &str)] = &[
    ("azureactivedirectory", "package-preview-2020-07"),
    ("resources", "package-policy-2020-03"),
    ("resources", "package-policy-2020-09"), // SchemaNotFound { ref_key: RefKey { file_path: "../azure-rest-api-specs/specification/resources/resource-manager/Microsoft.Authorization/stable/2020-09-01/dataPolicyManifests.json", name: "CloudError"
    ("recoveryservicesbackup", "package-2020-07"), // duplicate fn get_operation_status
    ("recoveryservicesbackup", "package-2020-10"), // duplicate fn get_operation_status
    ("network", "package-2017-03-30-only"),  // SchemaNotFound 2017-09-01/network.json SubResource
    ("synapse", "package-2019-06-01-preview"), // TODO #80 path parameters
    ("recoveryservicessiterecovery", "package-2016-08"), // duplicate package-2016-08 https://github.com/Azure/azure-rest-api-specs/pull/11287
    ("mediaservices", "package-2019-05-preview"), // invalid unicode character of a dash instead of a hyphen https://github.com/Azure/azure-rest-api-specs/pull/11576
    // datamigration, same error for all
    // SchemaNotFound MigrateSqlServerSqlDbTask.json ValidationStatus, but may be buried
    ("datamigration", "package-2018-07-15-preview"),
    ("datamigration", "package-2018-04-19"),
    ("datamigration", "package-2018-03-31-preview"),
    ("datamigration", "package-2018-03-15-preview"),
    ("datamigration", "package-2017-11-15-preview"),
    ("compute", "package-2020-10-01-preview"),      // TODO #81 DataType::File
    ("compute", "package-2020-10-01-preview-only"), // TODO #81 DataType::File
    ("authorization", "package-2018-05-01-preview"),
    ("marketplace", "package-composite-v1"),
    ("synapse", "package-2020-12-01"),
];

// becuse of recursive types, some properties have to be boxed
// https://github.com/ctaggart/autorust/issues/73
const BOX_PROPERTIES: &[(&str, &str, &str)] = &[
    // cost-management
    ("../azure-rest-api-specs/specification/cost-management/resource-manager/Microsoft.CostManagement/stable/2020-06-01/costmanagement.json", "ReportConfigFilter", "not"),
    ("../azure-rest-api-specs/specification/cost-management/resource-manager/Microsoft.CostManagement/stable/2020-06-01/costmanagement.json", "QueryFilter", "not"),
    // network
    ("../azure-rest-api-specs/specification/network/resource-manager/Microsoft.Network/stable/2020-07-01/publicIpAddress.json", "PublicIPAddressPropertiesFormat", "ipConfiguration"),
    ("../azure-rest-api-specs/specification/network/resource-manager/Microsoft.Network/stable/2020-08-01/publicIpAddress.json", "PublicIPAddressPropertiesFormat", "ipConfiguration"),
    // databox
    ("../azure-rest-api-specs/specification/databox/resource-manager/Microsoft.DataBox/stable/2020-11-01/databox.json", "transferFilterDetails", "include"),
    ("../azure-rest-api-specs/specification/databox/resource-manager/Microsoft.DataBox/stable/2020-11-01/databox.json", "transferAllDetails", "include"),
    // logic
    ("../azure-rest-api-specs/specification/logic/resource-manager/Microsoft.Logic/stable/2019-05-01/logic.json", "SwaggerSchema", "items"),
    // migrateprojects
    ("../azure-rest-api-specs/specification/migrateprojects/resource-manager/Microsoft.Migrate/preview/2018-09-01-preview/migrate.json", "IEdmNavigationProperty", "partner"),
    ("../azure-rest-api-specs/specification/migrateprojects/resource-manager/Microsoft.Migrate/preview/2018-09-01-preview/migrate.json", "IEdmStructuredType", "baseType"),
    // hardwaresecuritymodels
    ("../azure-rest-api-specs/specification/hardwaresecuritymodules/resource-manager/Microsoft.HardwareSecurityModules/preview/2018-10-31-preview/dedicatedhsm.json", "Error", "innererror"),
];

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("file name was not utf-8"))]
    FileNameNotUtf8Error {},
    IoError {
        source: std::io::Error,
    },
    PathError {
        source: path::Error,
    },
    CodegenError {
        source: autorust_codegen::Error,
    },
    CargoTomlError {
        source: cargo_toml::Error,
    },
    LibRsError {
        source: lib_rs::Error,
    },
    GetSpecFoldersError {
        source: autorust_codegen::Error,
    },
}

fn main() -> Result<()> {
    for (i, spec) in get_mgmt_configs().context(GetSpecFoldersError)?.iter().enumerate() {
        if ONLY_SERVICES.len() > 0 {
            if ONLY_SERVICES.contains(&spec.spec()) {
                println!("{} {}", i + 1, spec.spec());
                gen_crate(spec)?;
            }
        } else {
            if !SKIP_SERVICES.contains(&spec.spec()) {
                println!("{} {}", i + 1, spec.spec());
                gen_crate(spec)?;
            }
        }
    }
    Ok(())
}

fn gen_crate(spec: &SpecConfigs) -> Result<()> {
    let service_name = &get_service_name(spec.spec());
    let crate_name = &format!("azure_mgmt_{}", service_name);
    let output_folder = &path::join(OUTPUT_FOLDER, service_name).context(PathError)?;

    let src_folder = path::join(output_folder, "src").context(PathError)?;
    if src_folder.exists() {
        fs::remove_dir_all(&src_folder).context(IoError)?;
    }

    let mut feature_mod_names = Vec::new();
    let skip_service_tags: HashSet<&(&str, &str)> = SKIP_SERVICE_TAGS.iter().collect();

    let mut box_properties = HashSet::new();
    for (file_path, schema_name, property_name) in BOX_PROPERTIES {
        box_properties.insert(PropertyName {
            file_path: PathBuf::from(file_path),
            schema_name: schema_name.to_string(),
            property_name: property_name.to_string(),
        });
    }

    for config in spec.configs() {
        let tag = config.tag.as_str();
        if let Some(api_version) = to_api_version(&config) {
            if skip_service_tags.contains(&(spec.spec(), tag)) {
                // println!("  skipping {}", tag);
                continue;
            }
            println!("  {}", tag);
            // println!("  {}", api_version);
            let mod_name = &to_mod_name(tag);
            feature_mod_names.push((tag.to_string(), mod_name.clone()));
            // println!("  {}", mod_name);
            let mod_output_folder = path::join(&src_folder, mod_name).context(PathError)?;
            // println!("  {:?}", mod_output_folder);
            // for input_file in &config.input_files {
            //     println!("  {}", input_file);
            // }
            let input_files: Result<Vec<_>> = config
                .input_files
                .iter()
                .map(|input_file| Ok(path::join(spec.readme(), input_file).context(PathError)?))
                .collect();
            let input_files = input_files?;
            // for input_file in &input_files {
            //     println!("  {:?}", input_file);
            // }
            autorust_codegen::run(Config {
                api_version: Some(api_version),
                output_folder: mod_output_folder.into(),
                input_files,
                box_properties: box_properties.clone(),
            })
            .context(CodegenError)?;
        }
    }
    if feature_mod_names.len() == 0 {
        return Ok(());
    }
    cargo_toml::create(
        crate_name,
        &feature_mod_names,
        &path::join(output_folder, "Cargo.toml").context(PathError)?,
    )
    .context(CargoTomlError)?;
    lib_rs::create(&feature_mod_names, &path::join(src_folder, "lib.rs").context(PathError)?).context(LibRsError)?;

    Ok(())
}

fn get_service_name(spec_folder: &str) -> String {
    spec_folder.to_snake_case().replace("-", "_")
}
