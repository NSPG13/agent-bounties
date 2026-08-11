// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "./IAgentBounty.sol";

/// @notice Fixed-price, one-ticket-per-wallet-and-role stake registry.
/// It records protocol accounts, stake, availability and protocol events only;
/// a ticket is not identity or proof that two wallets have unrelated owners.
contract AnonymousStakePoolV1 {
    using SafeBountyToken for address;

    enum Role {
        Solver,
        Verifier
    }

    struct Ticket {
        uint128 stake;
        uint128 locked;
        uint64 activationAt;
        uint64 unbondAt;
        bool active;
        bool available;
    }

    uint256 public constant STAKE_AMOUNT = 5_000_000;
    uint64 public constant ACTIVATION_DELAY = 7 days;
    uint64 public constant UNBONDING_DELAY = 7 days;
    uint256 public constant MAX_ACTIVE_TICKETS = 64;
    uint256 public constant MINIMUM_VERIFIER_TICKETS = 8;

    address public immutable settlementToken;
    address public immutable controller;

    mapping(address => mapping(Role => Ticket)) public tickets;
    mapping(bytes32 => mapping(address => mapping(Role => uint256))) public caseLocks;
    mapping(Role => address[]) private _activeWallets;
    mapping(Role => mapping(address => uint256)) private _activeIndexPlusOne;
    uint256 public totalActiveTickets;
    uint256 public reserveSlashed;

    event StakeRegistered(address indexed wallet, Role indexed role, uint256 amount, uint64 activationAt);
    event TicketActivated(address indexed wallet, Role indexed role);
    event AvailabilityChanged(address indexed wallet, Role indexed role, bool available);
    event UnbondingStarted(address indexed wallet, Role indexed role, uint64 unbondAt);
    event StakeWithdrawn(address indexed wallet, Role indexed role, uint256 amount);
    event StakeLocked(bytes32 indexed caseId, address indexed wallet, Role indexed role, uint256 amount);
    event StakeReleased(bytes32 indexed caseId, address indexed wallet, Role indexed role, uint256 amount);
    event StakeSlashed(
        bytes32 indexed caseId, address indexed wallet, Role indexed role, uint256 amount, address recipient
    );
    event StakeRestored(address indexed wallet, Role indexed role, uint256 amount, uint64 activationAt);

    modifier onlyController() {
        require(msg.sender == controller, "controller only");
        _;
    }

    constructor(address settlementToken_, address controller_) {
        require(settlementToken_ != address(0), "token zero");
        require(controller_ != address(0), "controller zero");
        settlementToken = settlementToken_;
        controller = controller_;
    }

    function register(Role role) external {
        Ticket storage ticket = tickets[msg.sender][role];
        require(ticket.stake == 0, "ticket exists");
        uint64 activationAt = uint64(block.timestamp) + ACTIVATION_DELAY;
        ticket.stake = uint128(STAKE_AMOUNT);
        ticket.activationAt = activationAt;
        settlementToken.safeTransferFrom(msg.sender, address(this), STAKE_AMOUNT);
        emit StakeRegistered(msg.sender, role, STAKE_AMOUNT, activationAt);
    }

    function restore(Role role) external {
        Ticket storage ticket = tickets[msg.sender][role];
        require(ticket.stake > 0 && ticket.stake < STAKE_AMOUNT, "restore unavailable");
        require(ticket.locked == 0 && ticket.unbondAt == 0, "ticket encumbered");
        uint256 amount = STAKE_AMOUNT - ticket.stake;
        ticket.stake = uint128(STAKE_AMOUNT);
        ticket.activationAt = uint64(block.timestamp) + ACTIVATION_DELAY;
        settlementToken.safeTransferFrom(msg.sender, address(this), amount);
        emit StakeRestored(msg.sender, role, amount, ticket.activationAt);
    }

    function activate(Role role) external {
        Ticket storage ticket = tickets[msg.sender][role];
        require(ticket.stake == STAKE_AMOUNT, "stake incomplete");
        require(!ticket.active && ticket.unbondAt == 0, "ticket unavailable");
        require(ticket.activationAt != 0 && block.timestamp >= ticket.activationAt, "activation pending");
        require(totalActiveTickets < MAX_ACTIVE_TICKETS, "pool full");
        ticket.active = true;
        ticket.available = true;
        _activeWallets[role].push(msg.sender);
        _activeIndexPlusOne[role][msg.sender] = _activeWallets[role].length;
        totalActiveTickets += 1;
        emit TicketActivated(msg.sender, role);
        emit AvailabilityChanged(msg.sender, role, true);
    }

    function setAvailability(Role role, bool available) external {
        Ticket storage ticket = tickets[msg.sender][role];
        require(ticket.active && ticket.unbondAt == 0, "ticket inactive");
        if (!available) require(ticket.locked == 0, "stake locked");
        ticket.available = available;
        emit AvailabilityChanged(msg.sender, role, available);
    }

    function beginUnbond(Role role) external {
        Ticket storage ticket = tickets[msg.sender][role];
        require(ticket.stake > 0 && ticket.unbondAt == 0, "unbond unavailable");
        require(ticket.locked == 0, "stake locked");
        _deactivate(msg.sender, role, ticket);
        ticket.unbondAt = uint64(block.timestamp) + UNBONDING_DELAY;
        emit UnbondingStarted(msg.sender, role, ticket.unbondAt);
    }

    function withdraw(Role role) external {
        Ticket storage ticket = tickets[msg.sender][role];
        require(ticket.unbondAt != 0 && block.timestamp >= ticket.unbondAt, "unbonding pending");
        require(ticket.locked == 0, "stake locked");
        uint256 amount = ticket.stake;
        require(amount > 0, "nothing to withdraw");
        delete tickets[msg.sender][role];
        settlementToken.safeTransfer(msg.sender, amount);
        emit StakeWithdrawn(msg.sender, role, amount);
    }

    function lock(bytes32 caseId, address wallet, Role role, uint256 amount) external onlyController {
        require(caseId != bytes32(0) && amount > 0, "lock invalid");
        Ticket storage ticket = tickets[wallet][role];
        require(ticket.active && ticket.available && ticket.unbondAt == 0, "ticket unavailable");
        require(caseLocks[caseId][wallet][role] == 0, "case already locked");
        require(amount <= ticket.stake - ticket.locked, "stake insufficient");
        caseLocks[caseId][wallet][role] = amount;
        ticket.locked += uint128(amount);
        emit StakeLocked(caseId, wallet, role, amount);
    }

    function release(bytes32 caseId, address wallet, Role role) external onlyController returns (uint256 amount) {
        amount = caseLocks[caseId][wallet][role];
        require(amount > 0, "lock missing");
        delete caseLocks[caseId][wallet][role];
        Ticket storage ticket = tickets[wallet][role];
        ticket.locked -= uint128(amount);
        emit StakeReleased(caseId, wallet, role, amount);
    }

    function slash(bytes32 caseId, address wallet, Role role, uint256 amount, address recipient)
        external
        onlyController
    {
        require(amount > 0 && recipient != address(0), "slash invalid");
        uint256 lockedForCase = caseLocks[caseId][wallet][role];
        require(amount <= lockedForCase, "slash exceeds lock");
        Ticket storage ticket = tickets[wallet][role];
        caseLocks[caseId][wallet][role] = lockedForCase - amount;
        ticket.locked -= uint128(amount);
        ticket.stake -= uint128(amount);
        if (ticket.stake < STAKE_AMOUNT) {
            _deactivate(wallet, role, ticket);
            ticket.activationAt = uint64(block.timestamp) + ACTIVATION_DELAY;
        }
        if (recipient == address(this)) {
            reserveSlashed += amount;
        } else {
            settlementToken.safeTransfer(recipient, amount);
        }
        emit StakeSlashed(caseId, wallet, role, amount, recipient);
    }

    function eligibleWallets(Role role, address[] calldata exclusions) external view returns (address[] memory result) {
        address[] storage active = _activeWallets[role];
        uint256 count;
        for (uint256 i = 0; i < active.length; i++) {
            if (_eligible(active[i], role, exclusions)) count += 1;
        }
        result = new address[](count);
        uint256 cursor;
        for (uint256 i = 0; i < active.length; i++) {
            address wallet = active[i];
            if (_eligible(wallet, role, exclusions)) {
                result[cursor] = wallet;
                cursor += 1;
            }
        }
    }

    function activeWalletCount(Role role) external view returns (uint256) {
        return _activeWallets[role].length;
    }

    function _eligible(address wallet, Role role, address[] calldata exclusions) private view returns (bool) {
        Ticket storage ticket = tickets[wallet][role];
        if (!ticket.active || !ticket.available || ticket.unbondAt != 0 || ticket.stake != STAKE_AMOUNT) return false;
        for (uint256 j = 0; j < exclusions.length; j++) {
            if (wallet == exclusions[j]) return false;
        }
        return true;
    }

    function _deactivate(address wallet, Role role, Ticket storage ticket) private {
        if (!ticket.active) {
            ticket.available = false;
            return;
        }
        uint256 indexPlusOne = _activeIndexPlusOne[role][wallet];
        require(indexPlusOne != 0, "active index missing");
        uint256 index = indexPlusOne - 1;
        uint256 lastIndex = _activeWallets[role].length - 1;
        if (index != lastIndex) {
            address moved = _activeWallets[role][lastIndex];
            _activeWallets[role][index] = moved;
            _activeIndexPlusOne[role][moved] = index + 1;
        }
        _activeWallets[role].pop();
        delete _activeIndexPlusOne[role][wallet];
        ticket.active = false;
        ticket.available = false;
        totalActiveTickets -= 1;
    }
}
