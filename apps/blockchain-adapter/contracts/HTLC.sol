// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract HTLC {
    struct Lock {
        address initiator;
        address recipient;
        bytes32 secretHash; // SHA256(preimage)
        uint256 amount;
        uint256 timelock; // unix timestamp
        bool claimed;
        bytes preimage;
    }

    mapping(bytes32 => Lock) public locks;

    event Locked(bytes32 indexed id, address indexed initiator, address indexed recipient, uint256 amount, uint256 timelock);
    event Claimed(bytes32 indexed id, address indexed claimer, bytes preimage);
    event Refunded(bytes32 indexed id, address indexed initiator);

    // Create a lock. 'id' must be unique per lock (caller can use keccak256(abi.encodePacked(...))).
    function lock(bytes32 id, address recipient, bytes32 secretHash, uint256 timelock) external payable {
        require(locks[id].amount == 0, "HTLC: id already exists");
        require(msg.value > 0, "HTLC: amount required");
        locks[id] = Lock({
            initiator: msg.sender,
            recipient: recipient,
            secretHash: secretHash,
            amount: msg.value,
            timelock: timelock,
            claimed: false,
            preimage: ""
        });
        emit Locked(id, msg.sender, recipient, msg.value, timelock);
    }

    function claim(bytes32 id, bytes calldata preimage) external {
        Lock storage l = locks[id];
        require(l.amount > 0, "HTLC: not found");
        require(!l.claimed, "HTLC: already claimed");
        // check secret hash matches (sha256)
        bytes32 h = sha256(preimage);
        require(h == l.secretHash, "HTLC: invalid preimage");
        require(msg.sender == l.recipient || msg.sender == l.initiator, "HTLC: not allowed");
        l.claimed = true;
        l.preimage = preimage;
        payable(l.recipient).transfer(l.amount);
        emit Claimed(id, msg.sender, preimage);
    }

    function refund(bytes32 id) external {
        Lock storage l = locks[id];
        require(l.amount > 0, "HTLC: not found");
        require(!l.claimed, "HTLC: already claimed");
        require(block.timestamp >= l.timelock, "HTLC: timelock not expired");
        uint256 amount = l.amount;
        l.amount = 0;
        l.claimed = true;
        payable(l.initiator).transfer(amount);
        emit Refunded(id, l.initiator);
    }

    // View helper
    function getLock(bytes32 id) external view returns (address, address, bytes32, uint256, uint256, bool) {
        Lock storage l = locks[id];
        return (l.initiator, l.recipient, l.secretHash, l.amount, l.timelock, l.claimed);
    }
}
