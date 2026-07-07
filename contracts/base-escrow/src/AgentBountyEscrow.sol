// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

interface IERC20 {
    function transferFrom(address from, address to, uint256 value) external returns (bool);
    function transfer(address to, uint256 value) external returns (bool);
}

contract AgentBountyEscrow {
    enum EscrowStatus {
        None,
        Funded,
        Disputed,
        Released,
        Refunded
    }

    struct Escrow {
        bytes32 bountyId;
        address payer;
        address token;
        uint256 amount;
        bytes32 termsHash;
        EscrowStatus status;
    }

    address public owner;
    address public settlementSigner;
    bool public paused;
    uint256 public nextEscrowId = 1;
    mapping(uint256 => Escrow) public escrows;

    event EscrowCreated(uint256 indexed escrowId, bytes32 indexed bountyId, address indexed payer, address token, uint256 amount, bytes32 termsHash);
    event EscrowReleased(uint256 indexed escrowId, bytes32 proofHash);
    event EscrowRefunded(uint256 indexed escrowId, bytes32 reasonHash);
    event EscrowDisputed(uint256 indexed escrowId, bytes32 disputeHash);
    event Paused(bool paused);

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    modifier onlySettlementSigner() {
        require(msg.sender == settlementSigner, "not settlement signer");
        _;
    }

    modifier notPaused() {
        require(!paused, "paused");
        _;
    }

    constructor(address settlementSigner_) {
        owner = msg.sender;
        settlementSigner = settlementSigner_;
    }

    function setSettlementSigner(address settlementSigner_) external onlyOwner {
        settlementSigner = settlementSigner_;
    }

    function pause(bool paused_) external onlyOwner {
        paused = paused_;
        emit Paused(paused_);
    }

    function createEscrow(bytes32 bountyId, address token, uint256 amount, bytes32 termsHash) external notPaused returns (uint256 escrowId) {
        require(amount > 0, "amount zero");
        require(token != address(0), "token zero");

        escrowId = nextEscrowId++;
        escrows[escrowId] = Escrow({
            bountyId: bountyId,
            payer: msg.sender,
            token: token,
            amount: amount,
            termsHash: termsHash,
            status: EscrowStatus.Funded
        });

        require(IERC20(token).transferFrom(msg.sender, address(this), amount), "transferFrom failed");
        emit EscrowCreated(escrowId, bountyId, msg.sender, token, amount, termsHash);
    }

    function release(uint256 escrowId, address[] calldata recipients, uint256[] calldata amounts, bytes32 proofHash) external onlySettlementSigner {
        Escrow storage escrow = escrows[escrowId];
        require(escrow.status == EscrowStatus.Funded || escrow.status == EscrowStatus.Disputed, "not releasable");
        require(recipients.length == amounts.length && recipients.length > 0, "bad split");

        uint256 total;
        for (uint256 i = 0; i < amounts.length; i++) {
            require(recipients[i] != address(0), "recipient zero");
            total += amounts[i];
        }
        require(total == escrow.amount, "split mismatch");

        escrow.status = EscrowStatus.Released;
        for (uint256 i = 0; i < recipients.length; i++) {
            require(IERC20(escrow.token).transfer(recipients[i], amounts[i]), "transfer failed");
        }

        emit EscrowReleased(escrowId, proofHash);
    }

    function refund(uint256 escrowId, bytes32 reasonHash) external onlySettlementSigner {
        Escrow storage escrow = escrows[escrowId];
        require(escrow.status == EscrowStatus.Funded || escrow.status == EscrowStatus.Disputed, "not refundable");

        escrow.status = EscrowStatus.Refunded;
        require(IERC20(escrow.token).transfer(escrow.payer, escrow.amount), "refund failed");
        emit EscrowRefunded(escrowId, reasonHash);
    }

    function markDisputed(uint256 escrowId, bytes32 disputeHash) external onlySettlementSigner {
        Escrow storage escrow = escrows[escrowId];
        require(escrow.status == EscrowStatus.Funded, "not disputable");
        escrow.status = EscrowStatus.Disputed;
        emit EscrowDisputed(escrowId, disputeHash);
    }
}

