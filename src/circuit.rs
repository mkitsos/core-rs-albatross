use algebra::bls12_377::{Fq, G2Projective, Parameters as Bls12_377Parameters};
use algebra::sw6::Fr as SW6Fr;
use algebra_core::{AffineCurve, ProjectiveCurve};
use crypto_primitives::FixedLengthCRHGadget;
use r1cs_core::{ConstraintSynthesizer, ConstraintSystem, SynthesisError};
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::{G1Gadget, G2Gadget};
use r1cs_std::prelude::*;

use crate::constants::{EPOCH_LENGTH, G1_GENERATOR1, G2_GENERATOR, MAX_NON_SIGNERS};
use crate::gadgets::{
    calculate_state_hash, setup_crh, AllocConstantGadget, CRHGadget, CRHWindow, MacroBlockGadget,
    CRH,
};
use crate::macro_block::MacroBlock;
use crate::{end_cost_analysis, next_cost_analysis, start_cost_analysis};
use algebra_core::curves::models::bls12::Bls12Parameters;

pub struct Circuit {
    // Private inputs
    prev_keys: Vec<G2Projective>,
    block_number: u32,
    block: MacroBlock,

    // Public inputs
    state_hash: Vec<u32>,
}

impl Circuit {
    pub fn new(
        prev_keys: Vec<G2Projective>,
        block_number: u32,
        block: MacroBlock,
        state_hash: Vec<u32>,
    ) -> Self {
        Self {
            prev_keys,
            block_number,
            block,
            state_hash,
        }
    }
}

impl ConstraintSynthesizer<Fq> for Circuit {
    fn generate_constraints<CS: ConstraintSystem<Fq>>(
        self,
        cs: &mut CS,
    ) -> Result<(), SynthesisError> {
        // Allocate all the constants
        #[allow(unused_mut)]
        let mut cost = start_cost_analysis!(cs, || "Alloc constants");
        let epoch_length_var = UInt32::constant(EPOCH_LENGTH);
        let max_non_signers_var: FpGadget<SW6Fr> = AllocConstantGadget::alloc_const(
            cs.ns(|| "max non signers"),
            &SW6Fr::from(MAX_NON_SIGNERS),
        )?;
        let sig_generator_var: G2Gadget<Bls12_377Parameters> = AllocConstantGadget::alloc_const(
            cs.ns(|| "signature generator"),
            &G2Projective::prime_subgroup_generator(),
        )?;
        let sum_generator_g1_var: G1Gadget<Bls12_377Parameters> = AllocConstantGadget::alloc_const(
            cs.ns(|| "sum generator g1"),
            &G1_GENERATOR1.clone().into_projective(),
        )?;
        let sum_generator_g2_var: G2Gadget<Bls12_377Parameters> = AllocConstantGadget::alloc_const(
            cs.ns(|| "sum generator g2"),
            &G2_GENERATOR.clone().into_projective(),
        )?;
        let crh_block_parameters_var =
            <CRHGadget as FixedLengthCRHGadget<CRH<CRHWindow>, SW6Fr>>::ParametersGadget::alloc(
                &mut cs.ns(|| "CRH block parameters"),
                || Ok(setup_crh::<CRHWindow>()),
            )?;

        next_cost_analysis!(cs, cost, || "Alloc private inputs");
        let prev_keys_var = Vec::<G2Gadget<Bls12_377Parameters>>::alloc_const(
            cs.ns(|| "previous keys"),
            &self.prev_keys[..],
        )?;

        let block_number_var = UInt32::alloc(cs.ns(|| "block number"), Some(self.block_number))?;

        let block_var = MacroBlockGadget::alloc(cs.ns(|| "macro blocks"), || Ok(&self.block))?;

        next_cost_analysis!(cs, cost, || { "Alloc public inputs" });
        // TODO: Make it a public input
        let state_hash_var =
            Vec::<UInt32>::alloc(cs.ns(|| "state hash"), || Ok(&self.state_hash[..]))?;

        // Verify equality initial state hash
        next_cost_analysis!(cs, cost, || { "Verify initial state hash" });
        let reference_hash = calculate_state_hash(
            cs.ns(|| "calculate initial state hash"),
            &block_number_var,
            &block_var.public_keys,
        )?;
        // for i in 0..state_hash.len() {}
        //
        // verification_flag.enforce_equal(
        //     cs.ns(|| format!("verification flag == block flag {}", i + 1)),
        //     block_flag,
        // )?;
        // state_hash_var.enforce_equal(cs.ns(|| "initial state hash equality"), &reference_hash)?;

        // Verify block
        next_cost_analysis!(cs, cost, || "Verify block");
        block_var.verify(
            cs.ns(|| "verify block"),
            &prev_keys_var,
            &max_non_signers_var,
            &block_number_var,
            &sig_generator_var,
            &sum_generator_g1_var,
            &sum_generator_g2_var,
            &crh_block_parameters_var,
        )?;

        // Increment block number
        let new_block_number_var = UInt32::addmany(
            cs.ns(|| format!("increment block number")),
            &[block_number_var, epoch_length_var.clone()],
        )?;

        // TODO: verify equality output state hash
        // Finally verify that the last block's public key sum correspond to the public input.
        next_cost_analysis!(cs, cost, || "Public key equality with input");

        end_cost_analysis!(cs, cost);

        Ok(())
    }
}