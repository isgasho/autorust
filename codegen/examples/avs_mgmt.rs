// cargo run --example avs_mgmt
// https://github.com/Azure/azure-rest-api-specs/blob/master/specification/vmware/resource-manager/

use autorust_codegen::*;

fn main() -> Result<()> {
    let api_version = "2020-06-01";
    let output_folder = "../azure-sdk-for-rust/services/avs/mgmt/2020-03-20/src/";
    let input_files = [
        "../azure-rest-api-specs/specification/vmware/resource-manager/Microsoft.AVS/stable/2020-03-20/vmware.json",
    ];
    run(Config {
        api_version: Some(api_version.to_owned()),
        output_folder: output_folder.into(),
        input_files: input_files.iter().map(Into::into).collect(),
    })?;

    Ok(())
}