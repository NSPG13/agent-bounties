// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../script/BaseSepoliaStandingMetaV2Rehearsal.s.sol";

contract BaseSepoliaStandingMetaV2RehearsalHarness is BaseSepoliaStandingMetaV2Rehearsal {
    function assertCompletionBalanceDeltas(
        uint256 parentBalanceBefore,
        uint256 childBalanceBefore,
        uint256 parentBalanceAfter,
        uint256 childBalanceAfter
    ) external pure {
        _assertCompletionBalanceDeltas(
            parentBalanceBefore, childBalanceBefore, parentBalanceAfter, childBalanceAfter
        );
    }
}

contract BaseSepoliaStandingMetaV2RehearsalBalanceTest {
    BaseSepoliaStandingMetaV2RehearsalHarness private harness;

    function setUp() public {
        harness = new BaseSepoliaStandingMetaV2RehearsalHarness();
    }

    function testBalanceDeltasAllowPriorRehearsalPayouts() public view {
        harness.assertCompletionBalanceDeltas(3_400_000, 300_000, 3_300_000, 1_200_000);
    }

    function testAbsoluteBalanceAssumptionIsRejected() public {
        (bool ok,) = address(harness).call(
            abi.encodeCall(harness.assertCompletionBalanceDeltas, (3_400_000, 300_000, 1_000_000, 1_000_000))
        );
        require(!ok, "absolute-balance regression must revert");
    }
}
