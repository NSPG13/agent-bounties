#!/usr/bin/env python3
"""Add the active-parent-window regression for PR #536."""

from pathlib import Path


def main() -> None:
    path = Path("contracts/base-escrow/test/StandingMetaV4.t.sol")
    value = path.read_text(encoding="utf-8")
    marker = "    function _createParent() private returns (StandingMetaParentV4 parent) {\n"
    if value.count(marker) != 1:
        raise SystemExit("StandingMetaV4.t.sol: _createParent marker drift")
    test = """    function testTooLateChildAssignmentCannotConsumePreparedEscrow() public {
        StandingMetaParentV4 parent = _createParent();
        StandingMetaParentFactoryV4.ClaimAndCreateChildRequest memory request = _claimRequest(parent);
        address childAddress = parentSolver.claimAndCreate(parentFactory, address(parent), request);
        StandingMetaChildV4 child = StandingMetaChildV4(childAddress);
        uint256 requestId = vrf.nextRequestId() - 1;
        vrf.fulfill(requestId, 717);
        solverSortition.deriveRanking(requestId);
        parentFactory.activateChildDraw(address(parent));
        address selected = _selectedChildSolver(parent);

        uint256 lastSafeClaim = uint256(parent.claimExpiresAt()) - parentFactory.CHILD_WORK_WINDOW()
            - parentFactory.CHILD_VERIFICATION_WINDOW();
        vm.warp(lastSafeClaim + 1);
        StandingMetaParentFactoryV4.BondAuthorization memory bond =
            _bondAuthorization(keccak256("too-late-child-bond"));
        (bool claimed,) = address(selected)
            .call(abi.encodeCall(StandingMetaV4Actor.claimChild, (parentFactory, address(parent), bond)));
        require(!claimed, "too-late child assignment consumed prepared escrow");

        parentSolver.cancelExpiredChild(parentFactory, address(parent), parent.round());
        require(child.bountyStatus() == StandingMetaChildV4.Status.Cancelled, "unsafe-window child not recoverable");
    }

"""
    path.write_text(value.replace(marker, test + marker, 1), encoding="utf-8")


if __name__ == "__main__":
    main()
