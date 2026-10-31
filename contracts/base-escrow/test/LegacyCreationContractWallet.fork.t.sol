// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;
import "../src/AgentBountyFactory.sol";
import "./CreatorReviewBountyV1.fork.t.sol";

/// Local fixture; accepts exact owner ECDSA or a 66-byte 0x7f-prefixed encoding.
contract LegacyCreation1271 {
    address public immutable owner;
    constructor(address owner_) { owner = owner_; }
    function isValidSignature(bytes32 digest, bytes calldata signature) external view returns(bytes4) {
        if (signature.length == 66 && signature[0] == 0x7f) signature = signature[1:];
        if (signature.length != 65) return 0xffffffff;
        bytes32 r; bytes32 s; uint8 v;
        assembly { r := calldataload(signature.offset) s := calldataload(add(signature.offset,32)) v := byte(0,calldataload(add(signature.offset,64))) }
        return ecrecover(digest,v,r,s)==owner ? bytes4(0x1626ba7e) : bytes4(0xffffffff);
    }
}
interface CircleCancellation {
    function cancelAuthorization(address,bytes32,bytes calldata) external;
    function authorizationState(address,bytes32) external view returns(bool);
}
/// No broadcasts: exact currently deployed factory + native USDC on an in-memory fork.
contract LegacyCreationContractWalletForkTest {
    CreatorOpenVm private constant vm=CreatorOpenVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    CircleForkToken private constant token=CircleForkToken(0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913);
    AgentBountyFactory private constant factory=AgentBountyFactory(0x082C52131aaF0C56e76b075f895EAB6fcaB6d2F9);
    function authorization(uint256 key,address creator,address predicted,bytes32 nonce) private returns(AgentBountyFactory.FundingAuthorization memory a) {
        a.validBefore=block.timestamp+1 hours;a.nonce=nonce;
        bytes32 payload=keccak256(abi.encode(keccak256("TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"),creator,predicted,21_300_000,0,a.validBefore,nonce));
        (a.v,a.r,a.s)=vm.sign(key,keccak256(abi.encodePacked("\x19\x01",token.DOMAIN_SEPARATOR(),payload)));
    }
    function testForkNativeUSDCRevokesWideContractAuthorization() public {
        if(!CreatorForkVm(address(vm)).envOr("RUN_CREATOR_OPEN_FORK",false)){CreatorForkVm(address(vm)).skip(true);return;}
        vm.createSelectFork("https://base.drpc.org");
        uint256 key=uint256(keccak256("agent-bounties/legacy-wide-cancellation"));
        address wallet=address(new LegacyCreation1271(vm.addr(key)));
        bytes32 nonce=keccak256("local only cancelled nonce");
        bytes32 payload=keccak256(abi.encode(keccak256("CancelAuthorization(address authorizer,bytes32 nonce)"),wallet,nonce));
        (uint8 v,bytes32 r,bytes32 s)=vm.sign(key,keccak256(abi.encodePacked("\x19\x01",token.DOMAIN_SEPARATOR(),payload)));
        CircleCancellation(address(token)).cancelAuthorization(wallet,nonce,abi.encodePacked(hex"7f",r,s,v));
        require(CircleCancellation(address(token)).authorizationState(wallet,nonce),"not cancelled");
        require(wallet.balance==0,"customer gas");
    }
    function testForkLegacyFactoryAccepts65ByteContractWalletAuthorization() public {
        if(!CreatorForkVm(address(vm)).envOr("RUN_CREATOR_OPEN_FORK",false)){CreatorForkVm(address(vm)).skip(true);return;}
        vm.createSelectFork("https://base.drpc.org");
        require(factory.settlementToken()==address(token),"wrong canonical token");
        uint256 key=uint256(keccak256("agent-bounties/legacy-1271/20260929"));
        address creator=address(new LegacyCreation1271(vm.addr(key)));
        vm.prank(token.masterMinter());require(token.configureMinter(address(this),21_300_000),"fork minter");require(token.mint(creator,21_300_000),"fork token balance");
        AgentBountyFactory.CreateBountyParams memory p=AgentBountyFactory.CreateBountyParams(20_300_000,1_000_000,keccak256("terms"),keccak256("policy"),keccak256("criteria"),keccak256("benchmark"),keccak256("schema"),uint64(block.timestamp+30 days),1 days,2 days,AgentBounty.VerificationMode.SignedQuorum,address(0),address(0),1);
        address[] memory verifiers=new address[](1);verifiers[0]=creator;
        bytes32 nonce=keccak256("local legacy contract creator");address predicted=factory.predictBountyAddress(creator,p,verifiers,nonce);
        (address deployed,)=factory.createBountyWithAuthorization(creator,p,verifiers,21_300_000,nonce,authorization(key,creator,predicted,nonce));
        require(deployed==predicted&&factory.isCanonicalBounty(deployed),"wrong canonical creation");
        require(AgentBounty(deployed).creator()==creator&&AgentBounty(deployed).status()==uint8(AgentBounty.BountyStatus.Claimable),"wrong creator or readiness");
        require(token.balanceOf(deployed)==21_300_000&&token.balanceOf(creator)==0,"exact principal");require(creator.balance==0,"zero creator ETH");
    }
}
