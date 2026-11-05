// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.26;

/// @dev Local rehearsal only. Unlike the permissive protocol test token, this
/// fixture checks the actual EIP-3009 domain, signature, expiry and nonce.
contract SponsoredCreationToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(bytes32 => bool)) public authorizationState;
    mapping(address => mapping(address => uint256)) public allowance;
    event AuthorizationCanceled(address indexed authorizer, bytes32 indexed nonce);
    /// Emitted by plain `transfer`, as USDC does, so payout receipts can be verified.
    event Transfer(address indexed from, address indexed to, uint256 value);

    function cancelAuthorization(address authorizer, bytes32 nonce, uint8 v, bytes32 r, bytes32 s) external {
        _cancelAuthorization(authorizer,nonce,abi.encodePacked(r,s,v));
    }
    function cancelAuthorization(address authorizer, bytes32 nonce, bytes calldata signature) external {
        _cancelAuthorization(authorizer,nonce,signature);
    }
    function _cancelAuthorization(address authorizer, bytes32 nonce, bytes memory signature) private {
        require(!authorizationState[authorizer][nonce], "authorization used");
        bytes32 domain = keccak256(abi.encode(
            keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
            keccak256(bytes(block.chainid == 84532 ? "USDC" : "USD Coin")), keccak256("2"), block.chainid, address(this)
        ));
        bytes32 payload = keccak256(abi.encode(keccak256("CancelAuthorization(address authorizer,bytes32 nonce)"),authorizer,nonce));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01",domain,payload));
        if (authorizer.code.length==0) {
            require(signature.length==65,"signature length");
            bytes32 r; bytes32 s; uint8 v;
            assembly { r := mload(add(signature,32)) s := mload(add(signature,64)) v := byte(0,mload(add(signature,96))) }
            require(authorizer != address(0) && ecrecover(digest,v,r,s) == authorizer,"signature");
        } else {
            (bool ok, bytes memory result) = authorizer.staticcall(abi.encodeWithSignature("isValidSignature(bytes32,bytes)",digest,signature));
            require(ok && result.length==32 && abi.decode(result,(bytes4))==0x1626ba7e,"contract signature");
        }
        authorizationState[authorizer][nonce] = true;
        emit AuthorizationCanceled(authorizer,nonce);
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(allowance[from][msg.sender] >= amount && balanceOf[from] >= amount, "allowance or balance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function mint(address owner, uint256 amount) external {
        balanceOf[owner] += amount;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }

    function receiveWithAuthorization(address from, address to, uint256 value, uint256 validAfter, uint256 validBefore, bytes32 nonce, bytes calldata signature) external {
        require(msg.sender == to, "payee only");
        require(block.timestamp > validAfter && block.timestamp < validBefore, "expiry");
        require(!authorizationState[from][nonce], "authorization used");
        bytes32 domain = keccak256(abi.encode(keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"), keccak256(bytes(block.chainid == 84532 ? "USDC" : "USD Coin")), keccak256("2"), block.chainid, address(this)));
        bytes32 payload = keccak256(abi.encode(keccak256("ReceiveWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"), from,to,value,validAfter,validBefore,nonce));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01",domain,payload));
        if (from.code.length == 0) {
            require(signature.length == 65, "signature length");
            bytes32 r; bytes32 s; uint8 v;
            assembly { r := calldataload(signature.offset) s := calldataload(add(signature.offset,32)) v := byte(0,calldataload(add(signature.offset,64))) }
            require(from != address(0) && ecrecover(digest,v,r,s)==from, "signature");
        } else {
            (bool ok,bytes memory result)=from.staticcall(abi.encodeWithSignature("isValidSignature(bytes32,bytes)",digest,signature));
            require(ok && result.length==32 && abi.decode(result,(bytes4))==0x1626ba7e,"contract signature");
        }
        require(balanceOf[from]>=value,"balance");
        authorizationState[from][nonce]=true; balanceOf[from]-=value; balanceOf[to]+=value;
    }

    function transferWithAuthorization(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        require(block.timestamp > validAfter && block.timestamp < validBefore, "expiry");
        require(!authorizationState[from][nonce], "authorization used");
        bytes32 domain = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes(block.chainid == 84532 ? "USDC" : "USD Coin")),
                keccak256("2"),
                block.chainid,
                address(this)
            )
        );
        bytes32 payload = keccak256(
            abi.encode(
                keccak256(
                    "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)"
                ),
                from,
                to,
                value,
                validAfter,
                validBefore,
                nonce
            )
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", domain, payload));
        if (from.code.length == 0) {
            require(from != address(0) && ecrecover(digest,v,r,s) == from, "signature");
        } else {
            (bool ok, bytes memory result) = from.staticcall(abi.encodeWithSignature("isValidSignature(bytes32,bytes)", digest, abi.encodePacked(r,s,v)));
            require(ok && result.length == 32 && abi.decode(result,(bytes4)) == 0x1626ba7e, "contract signature");
        }
        require(balanceOf[from] >= value, "balance");
        authorizationState[from][nonce] = true;
        balanceOf[from] -= value;
        balanceOf[to] += value;
    }
}

/// @dev Local Anvil has no OP Stack oracle. Exercise both fee components with
/// nonzero deterministic quotes; live oracle behavior is tested separately.
contract SponsoredCreationFeeOracle {
    function getL1FeeUpperBound(uint256 size) external pure returns (uint256) {
        return size * 100;
    }

    function getOperatorFee(uint256 gas) external pure returns (uint256) {
        return gas;
    }
}
