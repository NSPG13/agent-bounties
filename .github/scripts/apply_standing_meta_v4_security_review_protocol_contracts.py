#!/usr/bin/env python3
"""Apply protocol-level contract fixes found during the PR #536 review."""

from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    value = file.read_text(encoding="utf-8")
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one exact match, found {count}: {old[:120]!r}")
    file.write_text(value.replace(old, new, 1), encoding="utf-8")


def main() -> None:
    # Bound solver and verifier capacity independently so one role cannot starve
    # the other while preserving the fixed 64-wallet iteration bound per role.
    replace_once(
        "contracts/base-escrow/src/AnonymousStakePoolV1.sol",
        '        require(totalActiveTickets < MAX_ACTIVE_TICKETS, "pool full");\n',
        '        require(_activeWallets[role].length < MAX_ACTIVE_TICKETS, "role pool full");\n',
    )

    # A child funded inside the parent claim is not independently cancellable.
    # Recovery is routed through the canonical factories after the parent round
    # is no longer active.
    replace_once(
        "contracts/base-escrow/src/StandingMetaChildV4.sol",
        """    function cancel() external nonReentrant {
        require(msg.sender == creator && (_status == Status.Open || _status == Status.Claimable), "not cancellable");
        _status = Status.Cancelled;
        emit BountyCancelled(bountyId, fundedAmount + timeoutBondPool);
    }
""",
        """    function cancel() external onlyFactory nonReentrant {
        require(_status == Status.Open || _status == Status.Claimable, "not cancellable");
        _status = Status.Cancelled;
        emit BountyCancelled(bountyId, fundedAmount + timeoutBondPool);
    }
""",
    )
    replace_once(
        "contracts/base-escrow/src/StandingMetaChildFactoryV4.sol",
        """    function predictChildAddress(
""",
        """    function cancelAuthorized(address childAddress) external {
        require(configured && msg.sender == parentFactory, "parent factory only");
        require(isCanonicalChild[childAddress], "child not canonical");
        StandingMetaChildV4(childAddress).cancel();
    }

    function predictChildAddress(
""",
    )

    # Resolve canonical-child provenance through the already-exposed immutable
    # child-factory getter instead of adding another public method to the size-
    # constrained parent factory.
    verifier_path = "contracts/base-escrow/src/AppealableVerifierV1.sol"
    replace_once(
        verifier_path,
        """interface ICanonicalAppealableChildRegistryV1 {
    function isCanonicalAppealableChild(address bounty) external view returns (bool);
}
""",
        """interface ICanonicalAppealableParentFactoryV1 {
    function standingMetaChildFactory() external view returns (address);
}

interface ICanonicalAppealableChildFactoryV1 {
    function isCanonicalChild(address bounty) external view returns (bool);
}
""",
    )
    replace_once(
        verifier_path,
        """        address registry = controller.standingMetaParentFactory();
        require(
            controller.configured() && registry.code.length > 0
                && ICanonicalAppealableChildRegistryV1(registry).isCanonicalAppealableChild(bounty),
            "bounty not canonical"
        );
""",
        """        address parentFactory = controller.standingMetaParentFactory();
        require(controller.configured() && parentFactory.code.length > 0, "bounty not canonical");
        address childFactory = ICanonicalAppealableParentFactoryV1(parentFactory).standingMetaChildFactory();
        require(
            childFactory.code.length > 0
                && ICanonicalAppealableChildFactoryV1(childFactory).isCanonicalChild(bounty),
            "bounty not canonical"
        );
""",
    )

    path = "contracts/base-escrow/src/StandingMetaParentFactoryV4.sol"
    replace_once(
        path,
        """    event ChildSolverPromoted(
        address indexed parent, uint64 indexed round, address indexed candidate, uint8 rank, bytes32 reason
    );
""",
        """    event ChildSolverPromoted(
        address indexed parent, uint64 indexed round, address indexed candidate, uint8 rank, bytes32 reason
    );
    event PreparedChildCancelled(
        address indexed parent, uint64 indexed round, address indexed child, address creator
    );
""",
    )
    replace_once(
        path,
        """    function isCanonicalAppealableChild(address bounty) external view returns (bool) {
        return standingMetaChildFactory.isCanonicalChild(bounty);
    }

""",
        "",
    )
    replace_once(
        path,
        """    function roundChild(address parent, uint64 parentRound) external view returns (address) {
""",
        """    function cancelExpiredChild(address parentAddress, uint64 parentRound) external nonReentrant {
        require(isCanonicalParent[parentAddress], "parent not canonical");
        StandingMetaParentV4 parent = StandingMetaParentV4(parentAddress);
        RoundData storage data = _rounds[parentAddress][parentRound];
        require(data.child != address(0), "prepared child missing");
        StandingMetaChildV4 child = StandingMetaChildV4(data.child);
        require(msg.sender == child.creator() && child.status() == CLAIMABLE_STATUS, "child not refundable");
        uint64 currentRound = parent.round();
        StandingMetaParentV4.Status parentStatus = parent.bountyStatus();
        require(
            currentRound > parentRound
                || (currentRound == parentRound
                    && (parentStatus == StandingMetaParentV4.Status.Claimable
                        || parentStatus == StandingMetaParentV4.Status.Cancelled)),
            "parent round still active"
        );
        standingMetaChildFactory.cancelAuthorized(data.child);
        emit PreparedChildCancelled(parentAddress, parentRound, data.child, msg.sender);
    }

    function roundChild(address parent, uint64 parentRound) external view returns (address) {
""",
    )

    # CREATE2 already makes an exact duplicate parent deployment impossible.
    # Remove the redundant used-ID storage added by the first patch pass and keep
    # the regression proving the native CREATE2 uniqueness invariant.
    replace_once(path, "    mapping(bytes32 => address) public parentByBountyId;\n", "")
    replace_once(
        path,
        '        require(parentByBountyId[bountyId] == address(0), "parent bounty id already used");\n',
        "",
    )
    replace_once(path, "        parentByBountyId[bountyId] = parentAddress;\n", "")


if __name__ == "__main__":
    main()
