// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

import "../src/CreatorReviewBountyFactoryV1.sol";

interface CreatorOpenVm {
    function addr(uint256) external returns (address);
    function sign(uint256, bytes32) external returns (uint8, bytes32, bytes32);
    function warp(uint256) external;
    function chainId(uint256) external;
    function expectRevert() external;
    function serializeBytes(string calldata, string calldata, bytes calldata) external returns (string memory);
    function serializeBytes32(string calldata, string calldata, bytes32) external returns (string memory);
    function serializeAddress(string calldata, string calldata, address) external returns (string memory);
    function writeJson(string calldata, string calldata) external;
    function createDir(string calldata, bool) external;
    function createSelectFork(string calldata) external returns (uint256);
    function prank(address) external;
}

/// @dev Independent token-side EIP-3009 verification, including payee binding.
contract CreatorOpenStrictToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(bytes32 => bool)) public used;
    bool public skipTransfer;
    address public callback;
    bytes public callbackData;
    bool public callbackSucceeded;

    function mint(address owner, uint256 amount) external {
        balanceOf[owner] += amount;
    }

    function setSkipTransfer(bool value) external {
        skipTransfer = value;
    }

    function setCallback(address target, bytes calldata data) external {
        callback = target;
        callbackData = data;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function digest(address from, address to, uint256 amount, uint256 after_, uint256 before_, bytes32 nonce)
        public
        view
        returns (bytes32)
    {
        bytes32 domain = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("USDC"),
                keccak256("2"),
                block.chainid,
                address(this)
            )
        );
        bytes32 payload = keccak256(
            abi.encode(
                keccak256(
                    "ReceiveWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
                ),
                from,
                to,
                amount,
                after_,
                before_,
                nonce
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domain, payload));
    }

    function receiveWithAuthorization(
        address from,
        address to,
        uint256 amount,
        uint256 after_,
        uint256 before_,
        bytes32 nonce,
        bytes calldata signature
    ) external {
        require(msg.sender == to, "payee only");
        require(block.timestamp > after_ && block.timestamp < before_, "expiry");
        require(!used[from][nonce], "nonce used");
        bytes32 signedHash = digest(from, to, amount, after_, before_, nonce);
        if (from.code.length > 0) {
            require(
                IERC1271(from).isValidSignature(signedHash, signature) == IERC1271.isValidSignature.selector,
                "contract signature"
            );
        } else {
            require(signature.length == 65, "length");
            bytes32 r;
            bytes32 s;
            uint8 v;
            assembly {
                r := calldataload(signature.offset)
                s := calldataload(add(signature.offset, 32))
                v := byte(0, calldataload(add(signature.offset, 64)))
            }
            require(from != address(0) && ecrecover(signedHash, v, r, s) == from, "signature");
        }
        require(balanceOf[from] >= amount, "balance");
        used[from][nonce] = true;
        if (callback != address(0)) (callbackSucceeded,) = callback.call(callbackData);
        if (!skipTransfer) {
            balanceOf[from] -= amount;
            balanceOf[to] += amount;
        }
    }
}

contract CreatorOpen1271 {
    address private immutable owner;

    constructor(address signer) {
        owner = signer;
    }

    function isValidSignature(bytes32 digest, bytes calldata signature) external view returns (bytes4) {
        if (signature.length != 66 || signature[65] != 0x01) return bytes4(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 32))
            v := byte(0, calldataload(add(signature.offset, 64)))
        }
        return ecrecover(digest, v, r, s) == owner ? this.isValidSignature.selector : bytes4(0);
    }
}

contract CreatorReviewBountyV1Test {
    CreatorOpenVm private constant vm = CreatorOpenVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 private constant CREATOR_KEY = 0xC0FFEE;
    CreatorOpenStrictToken private token;
    CreatorReviewBountyFactoryV1 private factory;
    CreatorReviewBountyV1 private bounty;
    CreatorReviewBountyV1.Config private config;
    address private creator;
    uint256 private nonce;

    function setUp() public {
        vm.chainId(84532);
        vm.warp(1_800_000_000);
        creator = vm.addr(CREATOR_KEY);
        token = new CreatorOpenStrictToken();
        factory = new CreatorReviewBountyFactoryV1(address(token));
        config = CreatorReviewBountyV1.Config(
            keccak256("bounty"),
            creator,
            20_300_000,
            1_000_000,
            keccak256("terms"),
            keccak256("criteria"),
            keccak256("schema"),
            uint64(block.timestamp + 90 days),
            2 days,
            8
        );
        token.mint(creator, 1_000_000_000_000);
        address predicted = factory.predict(config, bytes32(0));
        bounty = CreatorReviewBountyV1(
            factory.createWithAuthorization(config, bytes32(0), _authorization(CREATOR_KEY, predicted, 21_300_000))
        );
        require(address(bounty) == predicted && creator.balance == 0, "zero ETH creator and deterministic creation");
    }

    function _signature(uint256 key, bytes32 digest) private returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function _authorization(uint256 key, address to, uint256 amount)
        private
        returns (CreatorReviewBountyV1.TokenAuthorization memory a)
    {
        a.validBefore = block.timestamp + 1 hours;
        a.nonce = bytes32(++nonce);
        a.signature = _signature(key, token.digest(vm.addr(key), to, amount, 0, a.validBefore, a.nonce));
    }

    function _submission(uint256 key) private returns (CreatorReviewBountyV1.Submission memory s) {
        s = CreatorReviewBountyV1.Submission(
            vm.addr(key),
            keccak256(abi.encode(key, "result")),
            keccak256(abi.encode(key, "evidence")),
            bounty.termsHash(),
            bounty.revision(),
            block.timestamp + 1 hours
        );
    }

    function _submit(uint256 key) private returns (uint64) {
        return _submitChecked(key, false);
    }

    function _submitChecked(uint256 key, bool shouldRevert) private returns (uint64) {
        CreatorReviewBountyV1.Submission memory s = _submission(key);
        token.mint(s.solver, bounty.reviewReward());
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(key, address(bounty), bounty.reviewReward());
        bytes memory signature = _signature(key, bounty.submissionDigest(s, a.nonce));
        if (shouldRevert) vm.expectRevert();
        return bounty.submitAuthorized(s, a, signature);
    }

    function _review(uint64 id, bool passed) private {
        _reviewChecked(id, passed, false);
    }

    function _reviewChecked(uint64 id, bool passed, bool shouldRevert) private {
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 response = keccak256(abi.encode(id, passed));
        bytes memory signature = _signature(CREATOR_KEY, bounty.reviewDigest(id, passed, response, deadline));
        if (shouldRevert) vm.expectRevert();
        bounty.reviewAuthorized(id, passed, response, deadline, signature);
    }

    function _solvent() private view {
        require(
            token.balanceOf(address(bounty)) >= bounty.refundablePrincipal() + bounty.outstandingBonds(), "solvency"
        );
    }

    function testConcurrentEntriesFirstPassingVerdictPaysOnceAndRefundsOthers() public {
        uint64 first = _submit(11);
        uint64 second = _submit(12);
        uint64 third = _submit(13);
        _review(first, false);
        require(
            bounty.refundablePrincipal() == 21_300_000 && bounty.outstandingBonds() == 2_000_000,
            "rejection preserves funds"
        );
        _review(second, true);
        require(bounty.winner() == second && token.balanceOf(vm.addr(12)) == 21_300_000, "winner reward and bond");
        require(vm.addr(12).balance == 0 && creator.balance == 0, "customer ETH stays zero");
        bounty.refundBond(third);
        require(
            token.balanceOf(vm.addr(13)) == 1_000_000 && token.balanceOf(address(bounty)) == 0,
            "loser gets unreviewed bond"
        );
        _reviewChecked(third, true, true);
        vm.expectRevert();
        bounty.refundBond(third);
        vm.expectRevert();
        bounty.refundPrincipal();
        _solvent();
    }

    function testExpiredReviewReturnsBondWithoutPayingForWork() public {
        uint64 id = _submit(11);
        vm.warp(block.timestamp + 2 days + 1);
        bounty.refundBond(id);
        require(
            token.balanceOf(vm.addr(11)) == 1_000_000 && bounty.refundablePrincipal() == 21_300_000,
            "only bond returned"
        );
        _reviewChecked(id, true, true);
        require(_submit(11) > id, "same wallet can submit a new independent attempt");
        _solvent();
    }

    function testRewardIncreaseInvalidatesOldTermsBeforeFirstEntryOnly() public {
        CreatorReviewBountyV1.Submission memory stale = _submission(11);
        CreatorReviewBountyV1.TokenAuthorization memory bond = _authorization(11, address(bounty), 1_000_000);
        bytes memory oldSignature = _signature(11, bounty.submissionDigest(stale, bond.nonce));
        CreatorReviewBountyV1.TokenAuthorization memory add = _authorization(CREATOR_KEY, address(bounty), 10_000_000);
        bytes32 next = keccak256("new terms");
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory signature = _signature(CREATOR_KEY, bounty.increaseDigest(30_300_000, next, add.nonce, deadline));
        bounty.increaseRewardAuthorized(30_300_000, next, add, deadline, signature);
        require(bounty.revision() == 2 && bounty.refundablePrincipal() == 31_300_000, "atomic funded revision");
        vm.expectRevert();
        bounty.submitAuthorized(stale, bond, oldSignature);
        _submit(12);
        vm.expectRevert();
        bounty.increaseRewardAuthorized(30_300_000, next, add, deadline, signature);
        _solvent();
    }

    function testSubmissionWinsRaceAgainstPreSignedRewardIncrease() public {
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(CREATOR_KEY, address(bounty), 10_000_000);
        bytes32 next = keccak256("new terms");
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory signature = _signature(CREATOR_KEY, bounty.increaseDigest(30_300_000, next, a.nonce, deadline));
        _submit(11);
        vm.expectRevert();
        bounty.increaseRewardAuthorized(30_300_000, next, a, deadline, signature);
        require(!token.used(creator, a.nonce) && bounty.revision() == 1, "racing increase spends nothing");
    }

    function testSignedCancellationAndPermissionlessFixedRecipientRefund() public {
        uint256 beforeBalance = token.balanceOf(creator);
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory signature = _signature(CREATOR_KEY, bounty.cancelDigest(deadline));
        bounty.cancelAuthorized(deadline, signature);
        bounty.refundPrincipal();
        require(token.balanceOf(creator) == beforeBalance + 21_300_000, "only creator refunded");
        vm.expectRevert();
        bounty.refundPrincipal();
        vm.expectRevert();
        bounty.cancelAuthorized(deadline, signature);
        _solvent();
    }

    function testCannotCancelActiveWorkAndExpiryKeepsBondSolventInEitherOrder() public {
        uint256 deadline = block.timestamp + 1 hours;
        bytes memory signature = _signature(CREATOR_KEY, bounty.cancelDigest(deadline));
        uint64 id = _submit(11);
        vm.expectRevert();
        bounty.cancelAuthorized(deadline, signature);
        vm.expectRevert();
        bounty.expire();
        vm.warp(uint256(bounty.submissionDeadline()) + bounty.reviewWindow() + 1);
        bounty.expire();
        bounty.refundPrincipal();
        _solvent();
        require(token.balanceOf(address(bounty)) == 1_000_000, "bond protected from principal refund");
        bounty.refundBond(id);
        _solvent();
        require(token.balanceOf(address(bounty)) == 0, "all obligations returned");
    }

    function testWrongSignerFieldsDomainAndExpiredTransportCannotSpendBond() public {
        CreatorReviewBountyV1.Submission memory s = _submission(11);
        token.mint(s.solver, 1_000_000);
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(11, address(bounty), 1_000_000);
        bytes memory signature = _signature(11, bounty.submissionDigest(s, a.nonce));
        s.evidenceHash = keccak256("altered");
        vm.expectRevert();
        bounty.submitAuthorized(s, a, signature);
        s = _submission(11);
        vm.chainId(8453);
        vm.expectRevert();
        bounty.submitAuthorized(s, a, signature);
        vm.chainId(84532);
        vm.warp(block.timestamp + 1 hours + 1);
        vm.expectRevert();
        bounty.submitAuthorized(s, a, signature);
        require(token.balanceOf(s.solver) == 1_000_000 && bounty.entryCount() == 0, "no failed attempt takes funds");
    }

    function testPayeeOnlyTokenAuthorizationCannotBeConsumedByRelayer() public {
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(11, address(bounty), 1_000_000);
        token.mint(vm.addr(11), 1_000_000);
        address solver = vm.addr(11);
        vm.expectRevert();
        token.receiveWithAuthorization(
            solver, address(bounty), 1_000_000, a.validAfter, a.validBefore, a.nonce, a.signature
        );
        require(!token.used(vm.addr(11), a.nonce), "authorization remains usable by exact payee");
    }

    function testFactoryReplaysWithoutTransferAndChangedConfigCannotSpendAuthorization() public {
        address predicted = factory.predict(config, bytes32(uint256(1)));
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(CREATOR_KEY, predicted, 21_300_000);
        CreatorReviewBountyV1.Config memory changed = config;
        changed.termsHash = keccak256("tampered");
        address foreign = factory.predict(changed, bytes32(uint256(1)));
        vm.expectRevert();
        factory.createWithAuthorization(changed, bytes32(uint256(1)), a);
        require(foreign.code.length == 0 && !token.used(creator, a.nonce), "atomic failure");
        factory.createWithAuthorization(config, bytes32(uint256(1)), a);
        uint256 balance = token.balanceOf(creator);
        require(factory.createWithAuthorization(config, bytes32(uint256(1)), a) == predicted, "same address");
        require(token.balanceOf(creator) == balance, "no duplicate funding");
    }

    function testQueueBoundIndependentSolverAndDuplicatePendingEntry() public {
        _submit(11);
        _submitChecked(11, true);
        _submitChecked(CREATOR_KEY, true);
        for (uint256 i = 12; i < 19; i++) {
            _submit(i);
        }
        _submitChecked(19, true);
        require(bounty.pendingEntries() == 8, "queue bounded");
        _solvent();
    }

    function testFundingMustActuallyArriveAndFailedFundingDoesNotConsumeNonce() public {
        CreatorReviewBountyV1.Submission memory s = _submission(11);
        token.mint(s.solver, 1_000_000);
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(11, address(bounty), 1_000_000);
        bytes memory signature = _signature(11, bounty.submissionDigest(s, a.nonce));
        token.setSkipTransfer(true);
        vm.expectRevert();
        bounty.submitAuthorized(s, a, signature);
        require(!token.used(s.solver, a.nonce) && bounty.entryCount() == 0, "no-op transfer reverts all accounting");
        token.setSkipTransfer(false);
        bounty.submitAuthorized(s, a, signature);
        _solvent();
    }

    function testTokenCallbackCannotReenterAnEntryOrRefund() public {
        token.setCallback(address(bounty), abi.encodeCall(bounty.refundPrincipal, ()));
        _submit(11);
        require(!token.callbackSucceeded(), "token callback cannot reenter");
        _solvent();
    }

    function testWorkDeadlineAboveThirtyDaysAndSignatureStillShort() public {
        require(bounty.submissionDeadline() > block.timestamp + 30 days, "long creator work window");
        CreatorReviewBountyV1.Submission memory s = _submission(11);
        s.deadline = block.timestamp + 3 hours;
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(11, address(bounty), 1_000_000);
        bytes memory signature = _signature(11, bounty.submissionDigest(s, a.nonce));
        vm.expectRevert();
        bounty.submitAuthorized(s, a, signature);
        CreatorReviewBountyV1.Config memory changed = config;
        changed.submissionDeadline = uint64(block.timestamp + 367 days);
        address predicted = factory.predict(changed, bytes32(0));
        a = _authorization(CREATOR_KEY, predicted, 21_300_000);
        vm.expectRevert();
        factory.createWithAuthorization(changed, bytes32(0), a);
    }

    function _contractAuthorization(uint256 key, address from, address to, uint256 amount)
        private
        returns (CreatorReviewBountyV1.TokenAuthorization memory a)
    {
        a.validBefore = block.timestamp + 1 hours;
        a.nonce = bytes32(++nonce);
        a.signature =
            abi.encodePacked(_signature(key, token.digest(from, to, amount, 0, a.validBefore, a.nonce)), hex"01");
    }

    function testContractWalletCreatorAndSolverUseFullLengthSignaturesWithoutETH() public {
        address contractCreator = address(new CreatorOpen1271(creator));
        address contractSolver = address(new CreatorOpen1271(vm.addr(11)));
        config.creator = contractCreator;
        token.mint(contractCreator, 21_300_000);
        address predicted = factory.predict(config, bytes32(0));
        bounty = CreatorReviewBountyV1(
            factory.createWithAuthorization(
                config, bytes32(0), _contractAuthorization(CREATOR_KEY, contractCreator, predicted, 21_300_000)
            )
        );
        CreatorReviewBountyV1.Submission memory s = _submission(11);
        s.solver = contractSolver;
        token.mint(contractSolver, 1_000_000);
        CreatorReviewBountyV1.TokenAuthorization memory a =
            _contractAuthorization(11, contractSolver, predicted, 1_000_000);
        bytes memory signature = abi.encodePacked(_signature(11, bounty.submissionDigest(s, a.nonce)), hex"01");
        bytes memory invalid = abi.encodePacked(_signature(12, bounty.submissionDigest(s, a.nonce)), hex"01");
        vm.expectRevert();
        bounty.submitAuthorized(s, a, invalid);
        require(!token.used(contractSolver, a.nonce), "invalid contract signature never spends");
        uint64 id = bounty.submitAuthorized(s, a, signature);
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 response = keccak256("contract creator review");
        signature =
            abi.encodePacked(_signature(CREATOR_KEY, bounty.reviewDigest(id, true, response, deadline)), hex"01");
        bounty.reviewAuthorized(id, true, response, deadline, signature);
        require(
            token.balanceOf(contractSolver) == 21_300_000 && token.balanceOf(contractCreator) == 1_000_000,
            "contract wallet payouts"
        );
        require(contractCreator.balance == 0 && contractSolver.balance == 0, "no customer ETH");
        _solvent();
    }

    function testAlteredReviewAndCancellationCannotChangeSignedAuthority() public {
        uint64 id = _submit(11);
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 response = keccak256("rejected");
        bytes memory signature = _signature(CREATOR_KEY, bounty.reviewDigest(id, false, response, deadline));
        vm.expectRevert();
        bounty.reviewAuthorized(id, true, response, deadline, signature);
        vm.expectRevert();
        bounty.reviewAuthorized(id, false, keccak256("changed reason"), deadline, signature);
        vm.expectRevert();
        bounty.reviewAuthorized(id, false, response, deadline + 1, signature);
        require(bounty.pendingEntries() == 1 && bounty.winner() == 0, "no changed verdict");
        bounty.reviewAuthorized(id, false, response, deadline, signature);
        vm.expectRevert();
        bounty.reviewAuthorized(id, false, response, deadline, signature);
        _solvent();
    }

    function testDirectImplementationAndCloneInitializationAreForbidden() public {
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(CREATOR_KEY, address(bounty), 21_300_000);
        CreatorReviewBountyV1 implementation = CreatorReviewBountyV1(factory.implementation());
        vm.expectRevert();
        implementation.initializeAndFund(config, a);
        vm.expectRevert();
        bounty.initializeAndFund(config, a);
        require(bounty.refundablePrincipal() == 21_300_000, "configuration unchanged");
    }

    function testExportSolidityEncodingVectors() public {
        CreatorReviewBountyV1.Submission memory s = _submission(11);
        CreatorReviewBountyV1.TokenAuthorization memory a = _authorization(11, address(bounty), 1_000_000);
        _submit(11);
        bytes32 response = keccak256("vector response");
        bytes32 next = keccak256("vector terms");
        uint256 deadline = block.timestamp + 1 hours;
        vm.serializeBytes("creator-open", "config", abi.encode(config));
        vm.serializeAddress("creator-open", "factory", address(factory));
        vm.serializeAddress("creator-open", "implementation", factory.implementation());
        vm.serializeAddress("creator-open", "bounty", address(bounty));
        vm.serializeBytes("creator-open", "submission", abi.encode(s));
        vm.serializeBytes("creator-open", "tokenAuthorization", abi.encode(a));
        vm.serializeBytes32("creator-open", "domain", bounty.domainSeparator());
        vm.serializeBytes32("creator-open", "submissionDigest", bounty.submissionDigest(s, a.nonce));
        vm.serializeBytes32("creator-open", "reviewDigest", bounty.reviewDigest(1, true, response, deadline));
        vm.serializeBytes32(
            "creator-open", "increaseDigest", bounty.increaseDigest(30_300_000, next, a.nonce, deadline)
        );
        vm.serializeBytes32("creator-open", "cancelDigest", bounty.cancelDigest(deadline));
        vm.serializeBytes(
            "creator-open", "create", abi.encodeCall(factory.createWithAuthorization, (config, bytes32(0), a))
        );
        vm.serializeBytes("creator-open", "submit", abi.encodeCall(bounty.submitAuthorized, (s, a, hex"112233")));
        vm.serializeBytes(
            "creator-open",
            "review",
            abi.encodeCall(bounty.reviewAuthorized, (1, true, response, deadline, hex"112233"))
        );
        vm.serializeBytes(
            "creator-open",
            "increase",
            abi.encodeCall(bounty.increaseRewardAuthorized, (30_300_000, next, a, deadline, hex"112233"))
        );
        vm.serializeBytes("creator-open", "cancel", abi.encodeCall(bounty.cancelAuthorized, (deadline, hex"112233")));
        vm.serializeBytes("creator-open", "expire", abi.encodeCall(bounty.expire, ()));
        vm.serializeBytes("creator-open", "refundBond", abi.encodeCall(bounty.refundBond, (1)));
        string memory output =
            vm.serializeBytes("creator-open", "refundPrincipal", abi.encodeCall(bounty.refundPrincipal, ()));
        vm.createDir("../../target", true);
        vm.writeJson(output, "../../target/creator-open-vectors.json");
    }

    function testFuzzSolvencyAcrossArbitraryRejectedPrefix(uint8 rawCount, bool payWinner) public {
        uint64 count = uint64(rawCount % 8) + 1;
        for (uint64 i = 1; i <= count; i++) {
            _submit(100 + i);
        }
        for (uint64 i = 1; i < count; i++) {
            _review(i, false);
            _solvent();
        }
        if (payWinner) {
            _review(count, true);
        } else {
            vm.warp(block.timestamp + 2 days + 1);
            bounty.refundBond(count);
        }
        _solvent();
        require(bounty.pendingEntries() == 0 && bounty.outstandingBonds() == 0, "no stranded obligations");
        require(token.balanceOf(address(bounty)) == (payWinner ? 0 : 21_300_000), "exact principal remains or settles");
    }
}
