use std::fmt;
use std::time::Duration;

use sawtooth_sdk::consensus::engine::{BlockId, PeerId};

use crate::timing::Timeout;
use crate::config::PhaseQueenConfig;

/// Phases of the PBFT algorithm, in `Normal` mode
#[derive(Debug, PartialEq, PartialOrd, Clone, Serialize, Deserialize)]
pub enum PhaseQueenPhase {
    Idle,
    Exchange,
    QueenExchange,
    Finishing,
}

impl fmt::Display for PhaseQueenPhase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                PhaseQueenPhase::Idle => "Idle",
                PhaseQueenPhase::Exchange => "Exchange",
                PhaseQueenPhase::QueenExchange => "QueenExchange",
                PhaseQueenPhase::Finishing => "Finishing",
            },
        )
    }
}

impl fmt::Display for PhaseQueenState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "({}, seq {})",
            self.phase, self.seq_num,
        )
    }
}

/// Information about the PBFT algorithm's state
#[derive(Debug, Serialize, Deserialize)]
pub struct PhaseQueenState {
    /// This node's ID
    pub id: PeerId,

    /// This node order number for queening
    pub order: u64,

    /// This is the exchanged value for the algorithm 
    pub v: u8,

    /// Keep queen v if out of sequence
    /// if buf[0] == 1 then out of sequencem buf[1] keeps the value
    pub queen_buffer: [u8; 2],

    /// This is needed to store v values
    pub c: Vec<[u8; 2]>,

    /// This is needed to store the current k stage
    pub k: u64,

    /// The node's current sequence number
    pub seq_num: u64,

    /// The block ID of the node's current chain head
    pub chain_head: BlockId,

    /// Block id to accept
    pub decision_block: BlockId,

    /// Current phase of the algorithm
    pub phase: PhaseQueenPhase,

    /// List of members in the PBFT network, including this node
    pub member_ids: Vec<PeerId>,

    /// The maximum number of faulty nodes in the network
    pub f: u64,

    /// The base time to use for retrying with exponential backoff
    pub exponential_retry_base: Duration,

    /// The maximum time for retrying with exponential backoff
    pub exponential_retry_max: Duration,
}

impl PhaseQueenState {
    /// Construct the initial state for a PBFT node
    ///
    /// # Panics
    /// + If the network this node is on does not have enough nodes to be Byzantine fault tolernant
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(id: PeerId, head_block_num: u64, config: &PhaseQueenConfig) -> Self {

        // TODO PHASEQUEEN: questo va aggiornato con il numero minimo di nodi phasequeen
        // Maximum number of faulty nodes in this network. Panic if there are not enough nodes.
        
        let f = ((config.members.len() - 1) / 4) as u64;
        if f == 0 {
            panic!("This network does not contain enough nodes to be fault tolerant");
        }

        let order: u64 = config.members.clone().iter().position(|x| x == &id).unwrap() as u64;

        let mut c = Vec::new();
        for _ in 0..f+1 {
            c.push([0, 0]);
        }

        PhaseQueenState {
            id,
            order: order,
            v: 0,
            queen_buffer: [0, 0],
            c: c,
            k: 0,
            seq_num: head_block_num + 1,
            chain_head: BlockId::new(),
            decision_block: BlockId::new(),
            phase: PhaseQueenPhase::Idle,
            f,
            member_ids: config.members.clone(),
            exponential_retry_base: config.exponential_retry_base,
            exponential_retry_max: config.exponential_retry_max,
        }
    }

    pub fn reset_c(&mut self) {
        let mut c = Vec::new();
        for _ in 0..self.f + 1 {
            c.push([0, 0]);
        }
        self.c = c;
    }

    /// Switch to the desired phase if it is the next phase of the algorithm; if it is not the next
    /// phase, return an error
    pub fn switch_phase(&mut self, desired_phase: PhaseQueenPhase) -> bool {
        let next_phase = match self.phase {
            PhaseQueenPhase::Idle => PhaseQueenPhase::Exchange,
            PhaseQueenPhase::Exchange =>  PhaseQueenPhase::QueenExchange,
            PhaseQueenPhase::QueenExchange => {
                if self.k < self.f {
                    PhaseQueenPhase::Exchange
                } else { 
                    PhaseQueenPhase::Finishing
                }
            }
            PhaseQueenPhase::Finishing => PhaseQueenPhase::Idle,
        };

        if next_phase != desired_phase {
            error!("Node attempted to go to {} when in phase {}", desired_phase, self.phase);
            return false;
        } else {
            self.phase = desired_phase;
        }
        
        true
    }

}