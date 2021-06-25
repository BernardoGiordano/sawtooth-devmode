use sawtooth_sdk::consensus::engine::{Block, BlockId, PeerId, PeerInfo};
use sawtooth_sdk::consensus::service::Service;

use crate::config::{PhaseQueenConfig};
use crate::state::{PhaseQueenState, PhaseQueenPhase};

use rand;
use rand::Rng;

/// Contains the core logic of the PhaseQueen node
pub struct PhaseQueenNode {
    /// Used for interactions with the validator
    pub service: Box<dyn Service>,
    pub rng: rand::ThreadRng,
}

impl PhaseQueenNode {
    /// Construct a new PhaseQueen node
    ///
    /// If the node is the primary on start-up, it initializes a new block on the chain
    pub fn new(
        config: &PhaseQueenConfig,
        chain_head: Block,
        connected_peers: Vec<PeerInfo>,
        service: Box<dyn Service>,
        state: &mut PhaseQueenState,
    ) -> Self {
        let mut n = PhaseQueenNode {
            service: service,
            rng: rand::thread_rng(),
        };

        state.chain_head = chain_head.block_id.clone();

        // TODO: GENERALIZE: ONLY FIRST NODE CAN PROPOSE
        if state.order == 0 {
            n.service.initialize_block(None).unwrap_or_else(|err| {
                error!("Couldn't initialize block on startup due to error: {}", err)
            });
        }
        n
    }

    pub fn cancel_block(&mut self) {
        debug!("Canceling block");
        match self.service.cancel_block() {
            Ok(_) => {}
            Err(err) => {
                panic!("Failed to cancel block: {:?}", err);
            }
        };
    }

    /// At a regular interval, try to finalize a block when the primary is ready
    pub fn try_publish(&mut self, state: &mut PhaseQueenState) -> () {
        if state.phase != PhaseQueenPhase::Idle {
            return;
        }

        // TODO: GENERALIZE: ONLY FIRST NODE CAN PROPOSE
        if state.order != 0 {
            return;
        }

        info!("{}: Attempting to summarize block", state);

        let summary = self.service
                .summarize_block()
                .expect("Failed to summarize block");

        match self.service.finalize_block(create_consensus(&summary)) {
            Ok(block_id) => {
                info!("{}: Publishing block {}", state, hex::encode(&block_id));
            }
            Err(err) => { error!("Could not finalize block: {}", err); }
        }
    }

    fn broadcast_value(&mut self, message: &str, v: u8, state: &mut PhaseQueenState) {
        self.service
            .broadcast(message, vec![
                v, 
                state.k as u8, 
                state.seq_num as u8,
                self.rng.gen_range(0, 255), // some random values
                self.rng.gen_range(0, 255), // some random values
                self.rng.gen_range(0, 255)  // some random values
            ])
            .expect("Failed to broadcast value");

        self.on_peer_message(message, v, state);
    }

    // ---------- Methods for handling Updates from the Validator ----------

    pub fn on_block_new(&mut self, block: Block, state: &mut PhaseQueenState) -> bool {
        info!(
            "{}: Got BlockNew: {} / {}",
            state,
            block.block_num,
            hex::encode(&block.block_id)
        );
        trace!("Block details: {:?}", block);

        // Only future blocks should be considered since committed blocks are final
        if block.block_num < state.seq_num {
            self.service
                .fail_block(block.block_id.clone())
                .unwrap_or_else(|err| error!("Couldn't fail block due to error: {:?}", err));
            warn!(
                "Received block {:?} / {:?} that is older than the current sequence number: {:?}",
                block.block_num,
                hex::encode(&block.block_id),
                state.seq_num,
            );
            return true;
        }

        self.service
            .check_blocks(vec![block.block_id.clone()])
            .expect("Failed to check block");

        self.handle_block_new(block.block_id, state);
        
        true
    }

    pub fn on_block_valid(&mut self, block_id: BlockId, state: &mut PhaseQueenState) -> bool {
        info!("Got BlockValid: {}", hex::encode(&block_id));

        true
    }

    pub fn on_block_invalid(&mut self, block_id: BlockId) -> bool {
        info!("Got BlockInvalid: {}", hex::encode(&block_id));

        true
    }

    pub fn on_block_commit(&mut self, block_id: BlockId, state: &mut PhaseQueenState) -> bool {
        info!("Got BlockCommit: {}", hex::encode(&block_id));

        true
    }

    pub fn on_peer_connected(&mut self, peer_id: PeerId, state: &mut PhaseQueenState) -> bool {
        info!("Got PeerConnected: {:?}", hex::encode(&peer_id));

        true
    }

    pub fn on_peer_message(&mut self, message: &str, v: u8, state: &mut PhaseQueenState) -> bool {
        info!("Got PeerMessage with message={} and v={}", message, v);
        match message {
            "exchange" => {
                state.c[state.k as usize][v as usize] += 1;

                let mut arrived_values: u8 = state.c[state.k as usize][0] + state.c[state.k as usize][1];

                if arrived_values == state.member_ids.len() as u8 {
                    self.handle_exchange_finished(state);

                    if state.queen_buffer[0] == 1 {
                        // TODO: refactoring
                        if state.c[state.k as usize][state.v as usize] < state.member_ids.len() as u8 - state.f as u8 {
                            state.v = v;
                        }
                        self.handle_queen_exchange_finished(state);
                    }
                }
            }
            "queen_exchange" => {
                if state.phase != PhaseQueenPhase::QueenExchange {
                    state.queen_buffer[0] = 1;
                    state.queen_buffer[1] = v;
                    info!("QueenMessage received while in state={}", state);
                }
                else if state.c[state.k as usize][state.v as usize] < state.member_ids.len() as u8 - state.f as u8 {
                    state.v = v;
                }
                self.handle_queen_exchange_finished(state);
            }
            _ => { }
        }
        true
    }

    // ---------- Methods for handling state changes ----------

    pub fn handle_block_new(&mut self, block_id: BlockId, state: &mut PhaseQueenState) {
        state.switch_phase(PhaseQueenPhase::Exchange);

        state.decision_block = block_id;
        state.v = self.rng.gen_range(0, 2);

        self.broadcast_value("exchange", state.v, state);
    }

    pub fn handle_exchange_finished(&mut self, state: &mut PhaseQueenState) {
        state.switch_phase(PhaseQueenPhase::QueenExchange);

        state.v = if state.c[state.k as usize][1] > 2 * state.f as u8 { 1 } else { 0 };
        let is_queen: bool = state.k == state.order;

        if is_queen {
            self.broadcast_value("queen_exchange", state.v, state);
        }
    }

    pub fn handle_queen_exchange_finished(&mut self, state: &mut PhaseQueenState) {
        info!("[PHASEQUEEN] queen exchange has finished: state={}", state);

        if state.phase != PhaseQueenPhase::QueenExchange {
            return;
        }

        if state.k < state.f {
            state.k += 1;
            state.switch_phase(PhaseQueenPhase::Exchange);
            self.broadcast_value("exchange", state.v, state);
            warn!("MANDO IL NUOVO VALORE IN BROADCASTTTTTTTTTTTTT");
        }
        else {
            state.switch_phase(PhaseQueenPhase::Finishing);
            info!("[PHASEQUEEN] FINISHING: v={}", state.v);
            self.handle_decision(state);
        }
    }

    pub fn handle_decision(&mut self, state: &mut PhaseQueenState) {
        if state.v == 1 {
            info!("[PHASEQUEEN] Committing block {:?}", state.decision_block);
            self.service
                .commit_block(state.decision_block.clone())
                .expect("Failed to commit block");

            state.seq_num += 1;
            state.chain_head = state.decision_block.clone();
        } else {
            info!("[PHASEQUEEN]O Failing block {:?}", state.decision_block);
            self.service
                .fail_block(state.decision_block.clone())
                .expect("Failed to fail block");

            // TODO: GENERALIZE: ONLY FIRST NODE CAN PROPOSE
            // if state.order == 0 {
            //     self.cancel_block();
            // }
        }

        state.k = 0;
        state.queen_buffer = [0, 0];
        state.reset_c();

        // TODO: GENERALIZE: ONLY FIRST NODE CAN PROPOSE
        if state.order == 0 {
            self.service.initialize_block(None).unwrap_or_else(|err| {
                error!("Couldn't initialize block due to error: {}", err)
            });
        }

        state.switch_phase(PhaseQueenPhase::Idle);
    }

}

fn create_consensus(summary: &[u8]) -> Vec<u8> {
    let mut consensus: Vec<u8> = Vec::from(&b"Devmode"[..]);
    consensus.extend_from_slice(summary);
    consensus
}