// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

import "@openzeppelin/contracts/security/ReentrancyGuard.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

contract FirstValidSubmissionBounty is ReentrancyGuard, Ownable {
    struct Commitment {
        address submitter;
        bytes32 commitHash;
        uint256 timestamp;
        uint256 bond;
        bool revealed;
        bool refunded;
    }

    struct Bounty {
        address creator;
        address verifier;
        uint256 reward;
        uint256 entryBond;
        uint256 maxEntries;
        uint256 revealDeadline;
        uint256 verificationDeadline;
        uint256 entryCount;
        bool settled;
        address winner;
    }

    mapping(uint256 => Bounty) public bounties;
    mapping(uint256 => mapping(uint256 => Commitment)) public commitments;
    uint256 public bountyCount;

    event BountyCreated(uint256 indexed bountyId, address indexed creator, uint256 reward, uint256 entryBond, uint256 maxEntries);
    event CommitmentSubmitted(uint256 indexed bountyId, uint256 indexed commitmentId, address indexed submitter, bytes32 commitHash);
    event RevealSubmitted(uint256 indexed bountyId, uint256 indexed commitmentId, address indexed submitter);
    event BountySettled(uint256 indexed bountyId, address indexed winner, uint256 reward);
    event BondRefunded(uint256 indexed bountyId, uint256 indexed commitmentId, address indexed submitter, uint256 amount);

    function createBounty(
        address _verifier,
        uint256 _entryBond,
        uint256 _maxEntries,
        uint256 _revealWindow,
        uint256 _verificationWindow
    ) external payable returns (uint256) {
        require(msg.value > 0, "Reward required");
        require(_verifier != address(0), "Invalid verifier");
        require(_entryBond > 0, "Bond required");
        require(_maxEntries > 0 && _maxEntries <= 100, "Invalid max entries");
        require(_revealWindow > 0 && _revealWindow <= 7 days, "Invalid reveal window");
        require(_verificationWindow > 0 && _verificationWindow <= 30 days, "Invalid verification window");

        uint256 bountyId = bountyCount++;
        bounties[bountyId] = Bounty({
            creator: msg.sender,
            verifier: _verifier,
            reward: msg.value,
            entryBond: _entryBond,
            maxEntries: _maxEntries,
            revealDeadline: block.timestamp + _revealWindow,
            verificationDeadline: 0,
            entryCount: 0,
            settled: false,
            winner: address(0)
        });

        emit BountyCreated(bountyId, msg.sender, msg.value, _entryBond, _maxEntries);
        return bountyId;
    }

    function submitCommitment(uint256 _bountyId, bytes32 _commitHash) external payable nonReentrant {
        Bounty storage bounty = bounties[_bountyId];
        require(!bounty.settled, "Bounty settled");
        require(block.timestamp < bounty.revealDeadline, "Commit period ended");
        require(bounty.entryCount < bounty.maxEntries, "Max entries reached");
        require(msg.value == bounty.entryBond, "Incorrect bond");
        require(_commitHash != bytes32(0), "Invalid commit hash");

        uint256 commitmentId = bounty.entryCount++;
        commitments[_bountyId][commitmentId] = Commitment({
            submitter: msg.sender,
            commitHash: _commitHash,
            timestamp: block.timestamp,
            bond: msg.value,
            revealed: false,
            refunded: false
        });

        emit CommitmentSubmitted(_bountyId, commitmentId, msg.sender, _commitHash);
    }

    function reveal(uint256 _bountyId, uint256 _commitmentId, bytes memory _solution, bytes32 _salt) external nonReentrant {
        Bounty storage bounty = bounties[_bountyId];
        Commitment storage commitment = commitments[_bountyId][_commitmentId];

        require(!bounty.settled, "Bounty settled");
        require(block.timestamp >= bounty.revealDeadline, "Reveal period not started");
        require(commitment.submitter == msg.sender, "Not submitter");
        require(!commitment.revealed, "Already revealed");
        require(keccak256(abi.encodePacked(_solution, _salt)) == commitment.commitHash, "Invalid reveal");

        commitment.revealed = true;

        if (bounty.verificationDeadline == 0) {
            bounty.verificationDeadline = block.timestamp + (bounty.revealDeadline - bounties[_bountyId].revealDeadline + 7 days);
        }

        emit RevealSubmitted(_bountyId, _commitmentId, msg.sender);
    }

    function settleBounty(uint256 _bountyId, uint256 _winningCommitmentId) external nonReentrant {
        Bounty storage bounty = bounties[_bountyId];
        require(msg.sender == bounty.verifier, "Not verifier");
        require(!bounty.settled, "Already settled");
        require(bounty.verificationDeadline > 0, "No reveals yet");
        require(block.timestamp <= bounty.verificationDeadline, "Verification expired");

        Commitment storage winningCommitment = commitments[_bountyId][_winningCommitmentId];
        require(winningCommitment.revealed, "Winner not revealed");

        for (uint256 i = 0; i < _winningCommitmentId; i++) {
            Commitment storage earlier = commitments[_bountyId][i];
            require(!earlier.revealed || earlier.submitter == address(0), "Earlier valid submission exists");
        }

        bounty.settled = true;
        bounty.winner = winningCommitment.submitter;

        uint256 totalPayout = bounty.reward + winningCommitment.bond;
        (bool success, ) = winningCommitment.submitter.call{value: totalPayout}("");
        require(success, "Transfer failed");

        emit BountySettled(_bountyId, winningCommitment.submitter, totalPayout);
    }

    function refundBond(uint256 _bountyId, uint256 _commitmentId) external nonReentrant {
        Bounty storage bounty = bounties[_bountyId];
        Commitment storage commitment = commitments[_bountyId][_commitmentId];

        require(commitment.submitter == msg.sender, "Not submitter");
        require(!commitment.refunded, "Already refunded");
        require(
            bounty.settled ||
            (bounty.verificationDeadline > 0 && block.timestamp > bounty.verificationDeadline) ||
            (!commitment.revealed && block.timestamp > bounty.revealDeadline + 7 days),
            "Refund not available"
        );
        require(bounty.winner != msg.sender, "Winner cannot refund");

        commitment.refunded = true;
        uint256 refundAmount = commitment.bond;

        (bool success, ) = msg.sender.call{value: refundAmount}("");
        require(success, "Refund failed");

        emit BondRefunded(_bountyId, _commitmentId, msg.sender, refundAmount);
    }

    function expireBounty(uint256 _bountyId) external nonReentrant {
        Bounty storage bounty = bounties[_bountyId];
        require(!bounty.settled, "Already settled");
        require(
            (bounty.verificationDeadline > 0 && block.timestamp > bounty.verificationDeadline) ||
            (bounty.entryCount == 0 && block.timestamp > bounty.revealDeadline + 7 days),
            "Cannot expire yet"
        );

        bounty.settled = true;

        (bool success, ) = bounty.creator.call{value: bounty.reward}("");
        require(success, "Refund to creator failed");
    }
}
