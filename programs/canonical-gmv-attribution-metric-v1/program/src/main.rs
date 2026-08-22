#![no_main]

use competition_metric_core::{execute_canonical_gmv_program, CanonicalGmvProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<CanonicalGmvProgramInput>();
    let output = execute_canonical_gmv_program(&input)
        .expect("invalid canonical-gmv-attribution-metric-v1 input");
    sp1_zkvm::io::commit_slice(&output.journal);
}
