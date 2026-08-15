#![no_main]

use competition_metric_core::{
    execute_structured_artifact_program, StructuredArtifactProgramInput,
    StructuredArtifactProgramWireInput,
};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = StructuredArtifactProgramInput::from(
        sp1_zkvm::io::read::<StructuredArtifactProgramWireInput>(),
    );
    let output = execute_structured_artifact_program(&input)
        .expect("invalid structured-artifact-metric-v1 input");
    sp1_zkvm::io::commit_slice(&output.journal);
}
