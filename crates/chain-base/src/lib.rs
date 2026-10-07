pub fn validate_bounty_v2(bounty: &Bounty) -> Result<(), Error> {
    if bounty.is_router() {
        return Err(Error::NonCanonicalCaller);
    }
    // ... existing validation
    Ok(())
}
