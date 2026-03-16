use std::io::Result;
fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=parca/proto");
    println!("cargo:rerun-if-changed=googleapis");

    tonic_prost_build::configure().build_server(false).compile_protos(
        &[
            "parca/proto/parca/query/v1alpha1/query.proto",
            "parca/proto/parca/metastore/v1alpha1/metastore.proto",
            "parca/proto/parca/profilestore/v1alpha1/profilestore.proto",
        ],
        &["parca/proto", "googleapis"],
    )?;
    Ok(())
}
