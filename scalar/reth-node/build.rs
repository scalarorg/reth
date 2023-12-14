use std::error::Error;
use vergen::EmitBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=proto");
    build_consensus_service();
    build_vergen()?;
    Ok(())
}

fn build_consensus_service() {
    tonic_build::configure()
        .out_dir("src/proto")
        .compile(&["proto/consensus.proto"], &["proto"])
        .expect("Failed to compile proto(s)");
}
fn build_vergen() -> Result<(), Box<dyn Error>> {
    // Emit the instructions
    EmitBuilder::builder()
        .git_sha(true)
        .build_timestamp()
        .cargo_features()
        .cargo_target_triple()
        .emit()?;
    Ok(())
}
