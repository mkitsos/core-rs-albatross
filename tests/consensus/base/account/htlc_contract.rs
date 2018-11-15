use beserial::{Serialize, Deserialize};
use nimiq::consensus::base::account::{Account, AccountType, HashedTimeLockedContract};
use nimiq::consensus::base::account::htlc_contract::{AnyHash, ProofType, HashAlgorithm};
use nimiq::consensus::base::primitive::{Address, Coin, crypto::KeyPair, hash::{Blake2bHasher, Sha256Hasher, Hasher}};
use nimiq::consensus::base::transaction::{SignatureProof, Transaction, TransactionFlags};
use nimiq::consensus::networks::NetworkId;

const HTLC: &str = "00000000000000001b215589344cf570d36bec770825eae30b73213924786862babbdb05e7c4430612135eb2a836812303daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f01000296350000000000000001";

#[test]
fn it_can_deserialize_a_htlc() {
    let bytes: Vec<u8> = hex::decode(HTLC).unwrap();
    let htlc: HashedTimeLockedContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    assert_eq!(htlc.balance, Coin::ZERO);
    assert_eq!(htlc.hash_algorithm, HashAlgorithm::Sha256);
    assert_eq!(htlc.hash_count, 1);
    assert_eq!(htlc.hash_root, AnyHash::from("daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f"));
    assert_eq!(htlc.sender, Address::from("1b215589344cf570d36bec770825eae30b732139"));
    assert_eq!(htlc.recipient, Address::from("24786862babbdb05e7c4430612135eb2a8368123"));
    assert_eq!(htlc.timeout, 169525);
    assert_eq!(htlc.total_amount, Coin::from(1));
}

#[test]
fn it_can_serialize_a_htlc() {
    let bytes: Vec<u8> = hex::decode(HTLC).unwrap();
    let htlc: HashedTimeLockedContract = Deserialize::deserialize(&mut &bytes[..]).unwrap();
    let mut bytes2: Vec<u8> = Vec::with_capacity(htlc.serialized_size());
    let size = htlc.serialize(&mut bytes2).unwrap();
    assert_eq!(size, htlc.serialized_size());
    assert_eq!(hex::encode(bytes2), HTLC);
}

#[test]
fn it_can_verify_creation_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE * 2 + AnyHash::SIZE + 6);
    let sender = Address::from([0u8; 20]);
    let recipient = Address::from([0u8; 20]);
    sender.serialize(&mut data);
    recipient.serialize(&mut data);
    HashAlgorithm::Blake2b.serialize(&mut data);
    AnyHash::from([0u8; 32]).serialize(&mut data);
    Serialize::serialize(&2u8, &mut data);
    Serialize::serialize(&1000u32, &mut data);

    let mut transaction = Transaction::new_contract_creation(
        vec![],
        sender.clone(),
        AccountType::Basic,
        AccountType::HTLC,
        Coin::from(100),
        Coin::from(0),
        0,
        NetworkId::Dummy,
    );

    // Invalid data
    assert!(!HashedTimeLockedContract::verify_incoming_transaction(&transaction));
    transaction.data = data;

    // Invalid recipient
    assert!(!HashedTimeLockedContract::verify_incoming_transaction(&transaction));
    transaction.recipient = transaction.contract_creation_address();

    // Valid
    assert!(HashedTimeLockedContract::verify_incoming_transaction(&transaction));

    // Invalid transaction flags
    transaction.flags = TransactionFlags::empty();
    transaction.recipient = transaction.contract_creation_address();
    assert!(!HashedTimeLockedContract::verify_incoming_transaction(&transaction));
    transaction.flags = TransactionFlags::CONTRACT_CREATION;

    // Invalid recipient type
    transaction.recipient_type = AccountType::Basic;
    transaction.recipient = transaction.contract_creation_address();
    assert!(!HashedTimeLockedContract::verify_incoming_transaction(&transaction));
    transaction.recipient_type = AccountType::HTLC;

    // Hash algorithm argon2d
    transaction.data[40] = 2;
    transaction.recipient = transaction.contract_creation_address();
    assert!(!HashedTimeLockedContract::verify_incoming_transaction(&transaction));

    // Invalid hash algorithm
    transaction.data[40] = 200;
    transaction.recipient = transaction.contract_creation_address();
    assert!(!HashedTimeLockedContract::verify_incoming_transaction(&transaction));
    transaction.data[40] = 1;

    // Invalid zero hash count
    transaction.data[73] = 0;
    transaction.recipient = transaction.contract_creation_address();
    assert!(!HashedTimeLockedContract::verify_incoming_transaction(&transaction));
}

#[test]
fn it_can_create_contract_from_transaction() {
    let mut data: Vec<u8> = Vec::with_capacity(Address::SIZE * 2 + AnyHash::SIZE + 6);
    let sender = Address::from([0u8; 20]);
    let recipient = Address::from([0u8; 20]);
    sender.serialize(&mut data);
    recipient.serialize(&mut data);
    HashAlgorithm::Blake2b.serialize(&mut data);
    AnyHash::from([0u8; 32]).serialize(&mut data);
    Serialize::serialize(&2u8, &mut data);
    Serialize::serialize(&1000u32, &mut data);
    let transaction = Transaction::new_contract_creation(
        data,
        sender.clone(),
        AccountType::Basic,
        AccountType::HTLC,
        Coin::from(100),
        Coin::from(0),
        0,
        NetworkId::Dummy,
    );
    match HashedTimeLockedContract::create(Coin::from(0), &transaction, 0) {
        Ok(htlc) => {
            assert_eq!(htlc.balance, Coin::from(100));
            assert_eq!(htlc.sender, sender);
            assert_eq!(htlc.recipient, recipient);
            assert_eq!(htlc.hash_root, AnyHash::from([0u8; 32]));
            assert_eq!(htlc.hash_count, 2);
            assert_eq!(htlc.timeout, 1000);
        }
        Err(_) => assert!(false)
    }
}


#[test]
fn it_does_not_support_incoming_transactions() {
    let contract = HashedTimeLockedContract {
        balance: Coin::from(1000),
        sender: Address::from([1u8; 20]),
        recipient: Address::from([2u8; 20]),
        hash_algorithm: HashAlgorithm::Blake2b,
        hash_root: AnyHash::from([3u8; 32]),
        hash_count: 1,
        timeout: 100,
        total_amount: Coin::from(1000),
    };

    let mut tx = Transaction::new_basic(Address::from([1u8; 20]), Address::from([2u8; 20]), Coin::from(1), Coin::from(1000), 1, NetworkId::Dummy);
    tx.recipient_type = AccountType::HTLC;

    assert!(contract.with_incoming_transaction(&tx, 2).is_err());
    assert!(contract.without_incoming_transaction(&tx, 2).is_err());
}

#[test]
fn it_can_verify_valid_outgoing_transaction() {
    let key_pair = KeyPair::generate();
    let _addr = Address::from(&key_pair.public);
    let mut tx = Transaction {
        data: vec![],
        sender: Address::from([0u8; 20]),
        sender_type: AccountType::HTLC,
        recipient: Address::from([0u8; 20]),
        recipient_type: AccountType::Basic,
        value: Coin::from(100),
        fee: Coin::from(0),
        validity_start_height: 1,
        network_id: NetworkId::Dummy,
        flags: TransactionFlags::empty(),
        proof: vec![],
    };

    // regular: valid Blake-2b
    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    let mut proof = Vec::with_capacity(3 + 2 * AnyHash::SIZE + signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(&AnyHash::from(<[u8; 32]>::from(Blake2bHasher::default().digest(&[0u8; 32]))), &mut proof);
    Serialize::serialize(&AnyHash::from([0u8; 32]), &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;
    assert!(HashedTimeLockedContract::verify_outgoing_transaction(&tx));

    // regular: valid SHA-256
    proof = Vec::with_capacity(3 + 2 * AnyHash::SIZE + signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Sha256, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(&AnyHash::from(<[u8; 32]>::from(Sha256Hasher::default().digest(&[0u8; 32]))), &mut proof);
    Serialize::serialize(&AnyHash::from([0u8; 32]), &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;
    assert!(HashedTimeLockedContract::verify_outgoing_transaction(&tx));

    // regular: invalid hash
    tx.proof[35] = 1;
    assert!(!HashedTimeLockedContract::verify_outgoing_transaction(&tx));
    tx.proof[35] = 0;

    // regular: invalid algorithm
    tx.proof[1] = 99;
    proof = Vec::with_capacity(3 + 2 * AnyHash::SIZE + signature_proof.serialized_size());
    assert!(!HashedTimeLockedContract::verify_outgoing_transaction(&tx));
    tx.proof[1] = HashAlgorithm::Blake2b as u8;

    // regular: invalid signature
    tx.proof[68] = tx.proof[68] % 250 + 1;
    assert!(!HashedTimeLockedContract::verify_outgoing_transaction(&tx));

    // regular: invalid over-long
    proof = Vec::with_capacity(4 + 2 * AnyHash::SIZE + signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(&AnyHash::from(<[u8; 32]>::from(Blake2bHasher::default().digest(&[0u8; 32]))), &mut proof);
    Serialize::serialize(&AnyHash::from([0u8; 32]), &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    Serialize::serialize(&0u8, &mut proof);
    tx.proof = proof;
    assert!(!HashedTimeLockedContract::verify_outgoing_transaction(&tx));

    // early resolve: valid
    proof = Vec::with_capacity(1 + 2 * signature_proof.serialized_size());
    Serialize::serialize(&ProofType::EarlyResolve, &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;
    assert!(HashedTimeLockedContract::verify_outgoing_transaction(&tx));

    // timeout resolve: valid
    proof = Vec::with_capacity(1 + signature_proof.serialized_size());
    Serialize::serialize(&ProofType::TimeoutResolve, &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;
    assert!(HashedTimeLockedContract::verify_outgoing_transaction(&tx));
}

#[test]
fn it_can_apply_and_revert_regular_transfer() {
    let key_pair = KeyPair::generate();
    let recipient = Address::from(&key_pair.public);
    let pre_image = AnyHash::from([1u8; 32]);
    let hash_root = AnyHash::from(<[u8; 32]>::from(Blake2bHasher::default().digest(&pre_image.as_bytes())));

    let start_contract = HashedTimeLockedContract {
        balance: Coin::from(1000),
        sender: Address::from([0u8; 20]),
        recipient: recipient.clone(),
        hash_algorithm: HashAlgorithm::Blake2b,
        hash_root,
        hash_count: 1,
        timeout: 100,
        total_amount: Coin::from(1000),
    };

    let mut tx = Transaction {
        data: vec![],
        sender: Address::from([0u8; 20]),
        sender_type: AccountType::HTLC,
        recipient: recipient.clone(),
        recipient_type: AccountType::Basic,
        value: Coin::from(1000),
        fee: Coin::from(0),
        validity_start_height: 1,
        network_id: NetworkId::Dummy,
        flags: TransactionFlags::empty(),
        proof: vec![],
    };

    let signature = key_pair.sign(&tx.serialize_content()[..]);
    let signature_proof = SignatureProof::from(key_pair.public, signature);
    let mut proof = Vec::with_capacity(3 + 2 * AnyHash::SIZE + signature_proof.serialized_size());
    Serialize::serialize(&ProofType::RegularTransfer, &mut proof);
    Serialize::serialize(&HashAlgorithm::Blake2b, &mut proof);
    Serialize::serialize(&1u8, &mut proof);
    Serialize::serialize(&start_contract.hash_root, &mut proof);
    Serialize::serialize(&pre_image, &mut proof);
    Serialize::serialize(&signature_proof, &mut proof);
    tx.proof = proof;

    let mut contract = start_contract.with_outgoing_transaction(&tx, 1).unwrap();
    assert_eq!(contract.balance, Coin::from(0));
    contract = contract.without_outgoing_transaction(&tx, 1).unwrap();
    assert_eq!(contract, start_contract);
}