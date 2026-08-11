#![no_main]

use competition_metric_core::{execute_public_vector_program, PublicVectorProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<PublicVectorProgramInput>();
    let output = execute_public_vector_program(&input).expect("invalid public-vector-metric-v1 input");
    sp1_zkvm::io::commit_slice(&output.journal);
}
