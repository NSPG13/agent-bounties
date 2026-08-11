// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./CanonicalIndependentChildVerifierV3.sol";

/// @notice Atomically deploys the immutable profitable standing-meta-v3 policy components.
/// A failed component deployment reverts the whole bundle, leaving no partial policy.
contract StandingMetaV3Bundle {
    address public immutable canonicalFactory;
    ParticipantEligibilityRegistry public immutable participantRegistry;
    OnchainTermsRegistry public immutable termsRegistry;
    CanonicalIndependentChildVerifierV3 public immutable verifierModule;

    constructor(address canonicalFactory_, address attester, address verifierOne, address verifierTwo) {
        require(canonicalFactory_.code.length > 0, "factory missing");
        require(attester != address(0), "attester zero");
        require(
            verifierOne != address(0) && verifierTwo != address(0) && verifierOne != verifierTwo, "verifiers invalid"
        );

        address[] memory verifiers = new address[](2);
        verifiers[0] = verifierOne;
        verifiers[1] = verifierTwo;

        canonicalFactory = canonicalFactory_;
        participantRegistry = new ParticipantEligibilityRegistry(attester);
        termsRegistry = new OnchainTermsRegistry();
        verifierModule = new CanonicalIndependentChildVerifierV3(
            canonicalFactory_, address(participantRegistry), address(termsRegistry), keccak256(abi.encode(verifiers)), 2
        );
    }
}
