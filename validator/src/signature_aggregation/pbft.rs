use std::sync::Arc;
use std::collections::HashMap;
use std::fmt;

use parking_lot::RwLock;

use primitives::validators::Validators;
use block_albatross::{PbftPrepareMessage, PbftCommitMessage, SignedPbftPrepareMessage, SignedPbftCommitMessage};
use messages::Message;
use hash::Blake2bHash;
use network::Peer;
use collections::bitset::BitSet;

use handel::update::{LevelUpdate, LevelUpdateMessage};
use handel::evaluator::{Evaluator, WeightedVote};
use handel::protocol::Protocol;
use handel::store::{SignatureStore, ReplaceStore};
use handel::aggregation::Aggregation;
use handel::config::Config;
use handel::partitioner::BinomialPartitioner;
use handel::verifier::MultithreadedVerifier;
use handel::multisig::{Signature, MultiSignature, IndividualSignature};
use handel::identity::WeightRegistry;

use super::voting::{VotingProtocol, Tag, VotingEvaluator, VotingSender, VoteAggregation, ValidatorRegistry};
use crate::validator_agent::ValidatorAgent;



impl Tag for PbftPrepareMessage {
    fn create_level_update_message(&self, update: LevelUpdate) -> Message {
        Message::PbftPrepare(Box::new(update.with_tag(self.clone())))
    }
}

impl Tag for PbftCommitMessage {
    fn create_level_update_message(&self, update: LevelUpdate) -> Message {
        Message::PbftCommit(Box::new(update.with_tag(self.clone())))
    }
}


/// The protocol and aggregation for prepare is just a vote.
pub type PbftPrepareProtocol = VotingProtocol<PbftPrepareMessage>;


/// The commit evalutor has a inner voting evaluator, which it uses for the implementation of `evaluate`.
/// The `is_final` must be adapted to take into account the signers of the prepare aggregation
pub struct PbftCommitEvaluator {
    /// The inner evaluator for the commits - ignoring the `is_final`.
    commit_evaluator: VotingEvaluator,

    /// The prepare aggregation. We need this to access the evaluator and signature store of the
    /// prepare phase.
    prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>,
}

impl Evaluator for PbftCommitEvaluator {
    fn evaluate(&self, signature: &Signature, level: usize) -> usize {
        self.commit_evaluator.evaluate(signature, level)
    }

    fn is_final(&self, signature: &Signature) -> bool {
        signature.signers_bitset().intersection_size(&self.prepare_signers())
            >= self.commit_evaluator.threshold
    }
}

impl PbftCommitEvaluator {
    /// Returns the `BitSet` of signers of the prepare phase.
    fn prepare_signers(&self) -> BitSet {
        let store = self.prepare_aggregation.protocol.store();
        let store = store.read();
        // FIXME: I think this doesn't return the best combined signature.
        store.best(store.best_level())
            .map(|signature| signature.signers.clone())
            .unwrap_or_default()
    }
}


/// The generic protocol implementation for validator voting
pub struct PbftCommitProtocol {
    pub tag: PbftCommitMessage,

    store: Arc<RwLock<ReplaceStore<BinomialPartitioner>>>,

    /// The evaluator being used. This either just counts votes
    evaluator: Arc<PbftCommitEvaluator>,

    sender: Arc<VotingSender<PbftCommitMessage>>,

    prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>,
}

impl Protocol for PbftCommitProtocol {
    type Registry = ValidatorRegistry;
    type Verifier = MultithreadedVerifier<ValidatorRegistry>;
    type Store = ReplaceStore<BinomialPartitioner>;
    type Evaluator = PbftCommitEvaluator;
    type Partitioner = BinomialPartitioner;
    type Sender = VotingSender<PbftCommitMessage>;

    fn registry(&self) -> Arc<Self::Registry> {
        Arc::clone(&self.prepare_aggregation.protocol.registry())
    }

    fn verifier(&self) -> Arc<Self::Verifier> {
        Arc::clone(&self.prepare_aggregation.protocol.verifier())
    }

    fn store(&self) -> Arc<RwLock<Self::Store>> {
        Arc::clone(&self.store)
    }

    fn evaluator(&self) -> Arc<Self::Evaluator> {
        Arc::clone(&self.evaluator)
    }

    fn partitioner(&self) -> Arc<Self::Partitioner> {
        Arc::clone(&self.prepare_aggregation.protocol.partitioner())
    }

    fn sender(&self) -> Arc<Self::Sender> {
        Arc::clone(&self.sender)
    }

    fn node_id(&self) -> usize {
        self.prepare_aggregation.protocol.node_id
    }
}

impl PbftCommitProtocol {
    pub fn new(prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>) -> Self {
        let prepare_protocol = &prepare_aggregation.protocol;

        let tag = PbftCommitMessage::from(prepare_protocol.tag.block_hash.clone());
        let registry = Arc::clone(&prepare_protocol.registry());
        let partitioner = Arc::clone(&prepare_protocol.partitioner());
        let store = Arc::new(RwLock::new(ReplaceStore::new(Arc::clone(&partitioner))));
        let evaluator = Arc::new(PbftCommitEvaluator {
            commit_evaluator: VotingEvaluator::new(
                Arc::clone(&store),
                Arc::clone(&registry),
                Arc::clone(&partitioner),
                prepare_protocol.evaluator().threshold
            ),
            prepare_aggregation: Arc::clone(&prepare_aggregation),
        });
        // TODO: If we Arc the peer list, we don't need two copies. Or decouple the peer list from
        // the sender.
        let peers = Arc::clone(&prepare_protocol.sender().peers);
        let sender = Arc::new(VotingSender::new(tag.clone(), peers));

        Self {
            tag,
            store,
            evaluator,
            sender,
            prepare_aggregation,
        }
    }

    // TODO: Duplicate code, see `VotingProtocol::votes()`
    pub fn votes(&self) -> usize {
        let store = self.store.read();
        store.best(store.best_level())
            .map(|multisig| {
                self.registry().signature_weight(&Signature::Multi(multisig.clone()))
                    .unwrap_or_else(|| panic!("Unknown signers in signature: {:?}", multisig))
            })
            .unwrap_or(0)
    }
}

impl fmt::Debug for PbftCommitProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PbftCommitProtocol {{ node_id: {}, {:?} }}", self.node_id(), self.tag)
    }
}



pub struct PbftAggregation {
    prepare_aggregation: Arc<Aggregation<PbftPrepareProtocol>>,
    commit_aggregation: Arc<Aggregation<PbftCommitProtocol>>,
}


impl PbftAggregation {
    pub fn new(proposal_hash: Blake2bHash, node_id: usize, validators: Validators, peers: Arc<HashMap<usize, Arc<ValidatorAgent>>>, config: Option<Config>) -> Self {
        let config = config.unwrap_or_default();

        let prepare_protocol = PbftPrepareProtocol::new(
            PbftPrepareMessage::from(proposal_hash.clone()),
            node_id,
            validators,
            &config,
            peers,

        );
        let prepare_aggregation = Aggregation::new(prepare_protocol, config.clone());

        let commit_protocol = PbftCommitProtocol::new(Arc::clone(&prepare_aggregation));
        let commit_aggregation = Aggregation::new(commit_protocol, config);

        Self {
            prepare_aggregation,
            commit_aggregation,
        }
    }

    pub fn push_signed_prepare(&self, contribution: SignedPbftPrepareMessage) {
        // deconstruct signed view change
        let SignedPbftPrepareMessage {
            signature,
            message: tag,
            signer_idx: node_id,
        } = contribution;
        let node_id = node_id as usize;

        // panic if the contribution doesn't belong to this aggregation
        if self.prepare_aggregation.protocol.tag != tag {
            panic!("Submitting prepare for {:?}, but aggregation is for {:?}", tag, self.prepare_aggregation.protocol.tag);
        }

        // panic if the contribution is from a different node
        if self.prepare_aggregation.protocol.node_id != node_id {
            panic!("Submitting prepare for validator {}, but aggregation is running as validator {}", node_id, self.node_id());
        }

        self.prepare_aggregation.push_contribution(IndividualSignature::new(signature, node_id));
    }

    pub fn push_signed_commit(&self, contribution: SignedPbftCommitMessage) {
        // deconstruct signed view change
        let SignedPbftCommitMessage {
            signature,
            message: tag,
            signer_idx: node_id,
        } = contribution;
        let node_id = node_id as usize;

        // panic if the contribution doesn't belong to this aggregation
        if self.commit_aggregation.protocol.tag != tag {
            panic!("Submitting commit for {:?}, but aggregation is for {:?}", tag, self.commit_aggregation.protocol.tag);
        }

        // panic if the contribution is from a different node
        if self.prepare_aggregation.protocol.node_id != node_id {
            panic!("Submitting commit for validator {}, but aggregation is running as validator {}", node_id, self.node_id());
        }

        self.commit_aggregation.push_contribution(IndividualSignature::new(signature, node_id));
    }

    pub fn push_prepare_level_update(&self, level_update: LevelUpdateMessage<PbftPrepareMessage>) {
        if level_update.tag != self.prepare_aggregation.protocol.tag {
            panic!("Submitting level update for {:?}, but aggregation is for {:?}");
        }
        self.prepare_aggregation.push_update(level_update.update);
    }

    pub fn push_commit_level_update(&self, level_update: LevelUpdateMessage<PbftCommitMessage>) {
        if level_update.tag != self.commit_aggregation.protocol.tag {
            panic!("Submitting level update for {:?}, but aggregation is for {:?}");
        }
        self.commit_aggregation.push_update(level_update.update);
    }

    pub fn proposal_hash(&self) -> &Blake2bHash {
        assert_eq!(self.prepare_aggregation.protocol.tag.block_hash, self.commit_aggregation.protocol.tag.block_hash);
        &self.prepare_aggregation.protocol.tag.block_hash
    }

    pub fn node_id(&self) -> usize {
        self.prepare_aggregation.protocol.node_id
    }

    pub fn votes(&self) -> (usize, usize) {
        (self.prepare_aggregation.protocol.votes(), self.commit_aggregation.protocol.votes())
    }
}