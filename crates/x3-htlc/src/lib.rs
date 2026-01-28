#![cfg_attr(not(feature = "std"), no_std)]

//! X3 HTLC (Hash Time-Locked Contract) Infrastructure
//!
//! Cross-chain atomic swaps via hash-time-locked contracts.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │           X3 HTLC Cross-Chain Atomic Swaps                 │
//! │                                                             │
//! │  Alice (X3VM)              ┌─────────────────┐  Bob (EVM)   │
//! │  1. Generate secret P      │   HTLC State    │              │
//! │  2. Compute H=SHA256(P)    │                 │              │
//! │  3. Create HTLC(H, T, A)───│ HtlcCreate      │─────────────▶│
//! │     (lock X3 tokens)       │ state_id: u64   │ 4. Lock ETH  │
//! │                            │ owner: account  │ (matching)   │
//! │  5. Reveal P               │ secret_hash: H  │              │
//! │  6. Claim(P) ◀─────────────│ timelock: t     │◀─────────────┤
//! │     (unlock ETH proof)     │ amount: u128     │ 7. Claim ETH │
//! │                            │ status: locked  │ (reveal P)   │
//! │  8. Claim X3tokens         │ claimed_by: Bob │              │
//! │     (using proof)          │                 │              │
//! │                            │ or refund after │              │
//! │  If timeout:               │ timelock        │              │
//! │  Refund → Refund proof ─────│ Refund path     │──────────────▶│
//! │                            │                 │              │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Design Principles
//!
//! 1. **Deterministic**: Hash function is SHA-256 (Bitcoin-compatible)
//! 2. **Fair**: Claim requires proving knowledge of preimage
//! 3. **Atomic**: Either both sides succeed or both fail
//! 4. **Cross-chain**: Proofs relay between networks
//! 5. **Recoverable**: Refund after timelock if unclaimed

use codec::{Decode, Encode};
use scale_info::TypeInfo;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_runtime::traits::Zero;
use sp_std::vec::Vec;

/// HTLC State ID (uniquely identifies an HTLC instance)
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode, TypeInfo)]
pub struct HtlcId(pub u64);

impl HtlcId {
    pub fn new(id: u64) -> Self {
        HtlcId(id)
    }

    pub fn inner(self) -> u64 {
        self.0
    }
}

/// HTLC State Status
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode, TypeInfo)]
pub enum HtlcStatus {
    /// Locked and waiting for claim
    Locked,
    /// Successfully claimed with preimage
    Claimed,
    /// Refunded after timelock expiry
    Refunded,
    /// Expired (no claim or refund yet processed)
    Expired,
}

impl HtlcStatus {
    /// Convert to status code for VM
    pub fn code(self) -> u32 {
        match self {
            HtlcStatus::Locked => 0,
            HtlcStatus::Claimed => 1,
            HtlcStatus::Refunded => 2,
            HtlcStatus::Expired => 3,
        }
    }
}

/// HTLC State: On-chain representation of a hash-time-locked contract
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
pub struct HtlcState<AccountId, Balance, BlockNumber> {
    /// Unique identifier for this HTLC
    pub id: HtlcId,

    /// Account that created this HTLC
    pub initiator: AccountId,

    /// Account that can claim this HTLC
    pub recipient: AccountId,

    /// SHA-256 hash of the preimage secret
    pub secret_hash: H256,

    /// Amount locked in this HTLC
    pub amount: Balance,

    /// Block number when timelock expires
    pub timelock: BlockNumber,

    /// Current status of the HTLC
    pub status: HtlcStatus,

    /// Block when this HTLC was created
    pub created_at: BlockNumber,

    /// Account that actually claimed it (if claimed)
    pub claimed_by: Option<AccountId>,

    /// Block when it was claimed (if claimed)
    pub claimed_at: Option<BlockNumber>,
}

impl<AccountId, Balance, BlockNumber> HtlcState<AccountId, Balance, BlockNumber>
where
    Balance: Zero,
{
    /// Create a new locked HTLC
    pub fn new(
        id: HtlcId,
        initiator: AccountId,
        recipient: AccountId,
        secret_hash: H256,
        amount: Balance,
        timelock: BlockNumber,
        created_at: BlockNumber,
    ) -> Self {
        HtlcState {
            id,
            initiator,
            recipient,
            secret_hash,
            amount,
            timelock,
            status: HtlcStatus::Locked,
            created_at,
            claimed_by: None,
            claimed_at: None,
        }
    }

    /// Mark as claimed with preimage
    pub fn mark_claimed(&mut self, claimer: AccountId, claimed_at: BlockNumber) {
        self.status = HtlcStatus::Claimed;
        self.claimed_by = Some(claimer);
        self.claimed_at = Some(claimed_at);
    }

    /// Mark as refunded after timelock
    pub fn mark_refunded(&mut self) {
        self.status = HtlcStatus::Refunded;
    }

    /// Mark as expired
    pub fn mark_expired(&mut self) {
        self.status = HtlcStatus::Expired;
    }

    /// Check if HTLC is still claimable
    pub fn is_claimable(&self) -> bool {
        self.status == HtlcStatus::Locked
    }
}

/// HTLC Proof: Evidence of successful claim that can be relayed across chains
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
pub struct HtlcProof<AccountId> {
    /// ID of the HTLC being proven
    pub htlc_id: HtlcId,

    /// Chain where HTLC was created (chain_id)
    pub from_chain: u32,

    /// Target chain for relay
    pub to_chain: u32,

    /// Recipient on target chain
    pub recipient: AccountId,

    /// Amount being transferred
    pub amount: u128,

    /// Secret preimage (revealed during claim)
    pub preimage: Vec<u8>,

    /// Secret hash (for verification)
    pub secret_hash: H256,

    /// Block height of claim on source chain
    pub proved_at: u32,

    /// Proof signature/commitment
    pub proof_hash: H256,
}

impl<AccountId: Clone> HtlcProof<AccountId> {
    /// Verify that preimage matches secret hash (SHA-256)
    pub fn verify_preimage(&self) -> bool {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&self.preimage);
        let computed_hash = H256::from_slice(&hasher.finalize());
        computed_hash == self.secret_hash
    }

    /// Create proof hash for relay verification
    pub fn compute_proof_hash(&self) -> H256 {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.htlc_id.0.to_le_bytes());
        hasher.update(self.from_chain.to_le_bytes());
        hasher.update(self.to_chain.to_le_bytes());
        hasher.update(self.amount.to_le_bytes());
        hasher.update(&self.preimage);
        hasher.update(self.proved_at.to_le_bytes());
        H256::from_slice(&hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_preimage() {
        let pre = b"secret-1".to_vec();
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&pre);
        let secret_hash = H256::from_slice(&hasher.finalize());

        let proof = HtlcProof {
            htlc_id: HtlcId::new(1),
            from_chain: 1,
            to_chain: 2,
            recipient: "bob".to_string(),
            amount: 100u128,
            preimage: pre.clone(),
            secret_hash,
            proved_at: 123u32,
            proof_hash: H256::zero(),
        };

        assert!(proof.verify_preimage());
        let computed = proof.compute_proof_hash();
        assert_ne!(computed, H256::zero());
    }

    #[test]
    fn test_htlc_state_claim_refund() {
        let mut state = HtlcState::new(
            HtlcId::new(10),
            "alice".to_string(),
            "bob".to_string(),
            H256::zero(),
            100u128,
            100u32,
            1u32,
        );

        assert!(state.is_claimable());
        state.mark_claimed("bob".to_string(), 2u32);
        assert_eq!(state.status, HtlcStatus::Claimed);
        let mut state2 = HtlcState::new(
            HtlcId::new(11),
            "alice".to_string(),
            "bob".to_string(),
            H256::zero(),
            50u128,
            100u32,
            1u32,
        );
        state2.mark_refunded();
        assert_eq!(state2.status, HtlcStatus::Refunded);
    }
}

/// HTLC Claim Result
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
pub struct HtlcClaimResult {
    /// ID of the HTLC
    pub htlc_id: HtlcId,

    /// Whether claim was successful
    pub success: bool,

    /// Reason if failed
    pub reason: ClaimFailureReason,

    /// Proof of successful claim (if successful)
    pub proof: Option<Vec<u8>>,
}

/// HTLC Claim Failure Reason
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub enum ClaimFailureReason {
    /// HTLC not found
    NotFound,
    /// HTLC already claimed
    AlreadyClaimed,
    /// HTLC already refunded
    AlreadyRefunded,
    /// Secret preimage doesn't match hash
    InvalidPreimage,
    /// HTLC is locked and can't be claimed yet
    Locked,
    /// Caller is not the recipient
    UnauthorizedClaimer,
}

impl ClaimFailureReason {
    /// Convert to error code for VM
    pub fn code(self) -> u32 {
        match self {
            ClaimFailureReason::NotFound => 1,
            ClaimFailureReason::AlreadyClaimed => 2,
            ClaimFailureReason::AlreadyRefunded => 3,
            ClaimFailureReason::InvalidPreimage => 4,
            ClaimFailureReason::Locked => 5,
            ClaimFailureReason::UnauthorizedClaimer => 6,
        }
    }
}

/// HTLC Refund Result
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
pub struct HtlcRefundResult {
    /// ID of the HTLC
    pub htlc_id: HtlcId,

    /// Whether refund was successful
    pub success: bool,

    /// Reason if failed
    pub reason: RefundFailureReason,
}

/// HTLC Refund Failure Reason
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub enum RefundFailureReason {
    /// HTLC not found
    NotFound,
    /// Timelock hasn't expired yet
    TimelockNotExpired,
    /// HTLC already claimed
    AlreadyClaimed,
    /// HTLC already refunded
    AlreadyRefunded,
    /// Caller is not the initiator
    UnauthorizedRefunder,
}

impl RefundFailureReason {
    /// Convert to error code for VM
    pub fn code(self) -> u32 {
        match self {
            RefundFailureReason::NotFound => 1,
            RefundFailureReason::TimelockNotExpired => 2,
            RefundFailureReason::AlreadyClaimed => 3,
            RefundFailureReason::AlreadyRefunded => 4,
            RefundFailureReason::UnauthorizedRefunder => 5,
        }
    }
}

/// HTLC Hostcall Interface for X3VM
///
/// These are the low-level hostcalls that X3VM uses to manage HTLC state.
/// The higher-level opcodes (HTLC_CREATE, HTLC_CLAIM, etc.) call these through
/// the hostcall registry.
pub trait HtlcHostcall<AccountId, Balance, BlockNumber> {
    /// Create a new HTLC lock
    fn htlc_create(
        &mut self,
        initiator: AccountId,
        recipient: AccountId,
        secret_hash: H256,
        amount: Balance,
        timelock: BlockNumber,
    ) -> Result<HtlcId, HtlcCreateError>;

    /// Claim an HTLC with preimage
    fn htlc_claim(
        &mut self,
        htlc_id: HtlcId,
        claimer: AccountId,
        preimage: Vec<u8>,
        current_block: BlockNumber,
    ) -> Result<HtlcProof<AccountId>, HtlcClaimError>;

    /// Refund an HTLC after timelock expiry
    fn htlc_refund(
        &mut self,
        htlc_id: HtlcId,
        refunder: AccountId,
        current_block: BlockNumber,
    ) -> Result<(), HtlcRefundError>;

    /// Query HTLC status
    fn htlc_status(&self, htlc_id: HtlcId) -> Option<HtlcStatus>;

    /// Get HTLC details (for verification)
    fn htlc_get(
        &self,
        htlc_id: HtlcId,
    ) -> Option<HtlcState<AccountId, Balance, BlockNumber>>;
}

/// HTLC Create Error
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtlcCreateError {
    /// Invalid amount (zero)
    InvalidAmount,
    /// Timelock in past
    InvalidTimelock,
    /// Storage error
    StorageError,
}

/// HTLC Claim Error
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtlcClaimError {
    NotFound,
    AlreadyClaimed,
    AlreadyRefunded,
    InvalidPreimage,
    UnauthorizedClaimer,
    StorageError,
}

/// HTLC Refund Error
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtlcRefundError {
    NotFound,
    TimelockNotExpired,
    AlreadyClaimed,
    AlreadyRefunded,
    UnauthorizedRefunder,
    StorageError,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_htlc_status_codes() {
        assert_eq!(HtlcStatus::Locked.code(), 0);
        assert_eq!(HtlcStatus::Claimed.code(), 1);
        assert_eq!(HtlcStatus::Refunded.code(), 2);
        assert_eq!(HtlcStatus::Expired.code(), 3);
    }

    #[test]
    fn test_proof_verification() {
        use sha2::{Digest, Sha256};
        
        let preimage = b"test_secret".to_vec();
        let mut hasher = Sha256::new();
        hasher.update(&preimage);
        let secret_hash = H256::from_slice(&hasher.finalize());

        let proof = HtlcProof {
            htlc_id: HtlcId::new(1),
            from_chain: 1,
            to_chain: 2,
            recipient: 0u32,
            amount: 1000,
            preimage,
            secret_hash,
            proved_at: 100,
            proof_hash: H256::zero(),
        };

        assert!(proof.verify_preimage());
    }

    #[test]
    fn test_htlc_state_creation() {
        let state = HtlcState::new(
            HtlcId::new(1),
            0u32,
            1u32,
            H256::zero(),
            1000u128,
            1000u32,
            100u32,
        );

        assert_eq!(state.status, HtlcStatus::Locked);
        assert!(state.is_claimable());
        assert_eq!(state.claimed_by, None);
    }
}
