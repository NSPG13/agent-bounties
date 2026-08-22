#![no_main]

use competition_metric_core::{
    execute_forward_canonical_gmv_program, ForwardCanonicalGmvProgramInput,
};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<ForwardCanonicalGmvProgramInput>();
    let output = execute_forward_canonical_gmv_program(&input)
        .expect("invalid forward-canonical-gmv-attribution-metric-v2 input");
    sp1_zkvm::io::commit_slice(&output.journal);
}
