// Copyright 2019-2021 Manta Network.
// This file is part of pallet-manta-pay.
//
// pallet-manta-pay is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// pallet-manta-pay is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with pallet-manta-pay.  If not, see <http://www.gnu.org/licenses/>.

use crate::*;
use ark_bls12_381::Bls12_381;
use ark_crypto_primitives::{CommitmentScheme as ArkCommitmentScheme, FixedLengthCRH};
use ark_ed_on_bls12_381::Fq;
use ark_groth16::generate_random_parameters;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
use ark_serialize::CanonicalSerialize;
use ark_std::rand::{RngCore, SeedableRng};
use hkdf::Hkdf;
use manta_asset::*;
use manta_crypto::*;
use rand_chacha::ChaCha20Rng;
use sha2::Sha512Trunc256;

#[cfg(feature = "std")]
use std::{fs::File, io::prelude::*};

pub const TRANSFER_PK: VerificationKey = VerificationKey {
	data: &TRANSFER_VKBYTES,
};

pub const RECLAIM_PK: VerificationKey = VerificationKey {
	data: &RECLAIM_VKBYTES,
};

/// Generate the ZKP keys with a default seed, and write to
/// `transfer_pk.bin` and `reclaim_pk.bin`.
#[cfg(feature = "std")]
pub fn write_zkp_keys() {
	let hash_param_seed = [1u8; 32];
	let commit_param_seed = [2u8; 32];
	let seed = [3u8; 32];
	let rng_salt: [u8; 32] = [
		0x74, 0x68, 0x69, 0x73, 0x20, 0x69, 0x73, 0x20, 0x61, 0x20, 0x73, 0x65, 0x65, 0x64, 0x20,
		0x66, 0x6f, 0x72, 0x20, 0x6d, 0x61, 0x6e, 0x74, 0x61, 0x20, 0x7a, 0x6b, 0x20, 0x74, 0x65,
		0x73, 0x74,
	];
	let mut rng_seed = [0u8; 32];
	let digest = Hkdf::<Sha512Trunc256>::extract(Some(rng_salt.as_ref()), &seed);
	rng_seed.copy_from_slice(&digest.0[0..32]);

	let mut transfer_pk_bytes =
		manta_transfer_zkp_key_gen(&hash_param_seed, &commit_param_seed, &rng_seed);
	let mut file = File::create("transfer_pk.bin").unwrap();
	file.write_all(transfer_pk_bytes.as_mut()).unwrap();
	// println!("transfer circuit pk length: {}", transfer_pk_bytes.len());

	let mut reclaim_pk_bytes =
		manta_reclaim_zkp_key_gen(&hash_param_seed, &commit_param_seed, &rng_seed);
	let mut file = File::create("reclaim_pk.bin").unwrap();
	file.write_all(reclaim_pk_bytes.as_mut()).unwrap();
	// println!("reclaim circuit pk length: {}", reclaim_pk_bytes.len());
}

// Generate ZKP keys for `private_transfer` circuit.
#[cfg(feature = "std")]
fn manta_transfer_zkp_key_gen(
	hash_param_seed: &[u8; 32],
	commit_param_seed: &[u8; 32],
	rng_seed: &[u8; 32],
) -> Vec<u8> {
	// rebuild the parameters from the inputs
	let mut rng = ChaCha20Rng::from_seed(*commit_param_seed);
	let commit_param = CommitmentScheme::setup(&mut rng).unwrap();

	let mut rng = ChaCha20Rng::from_seed(*hash_param_seed);
	let hash_param = Hash::setup(&mut rng).unwrap();

	let mut rng = ChaCha20Rng::from_seed(*rng_seed);
	let mut coins = Vec::new();
	let mut ledger = Vec::new();
	let mut sk = [0u8; 32];

	for e in 0..128 {
		rng.fill_bytes(&mut sk);

		let sender = MantaAsset::sample(&commit_param, &sk, &TEST_ASSET, &(e + 100), &mut rng);
		ledger.push(sender.commitment);
		coins.push(sender);
	}

	// sender's total value is 210
	let sender_1 = coins[0].clone();
	let sender_2 = coins[10].clone();

	let sender_1 = SenderMetaData::build(hash_param.clone(), sender_1, &ledger);
	let sender_2 = SenderMetaData::build(hash_param.clone(), sender_2, &ledger);

	// receiver's total value is also 210
	rng.fill_bytes(&mut sk);
	let receiver_1_full =
		MantaAssetFullReceiver::sample(&commit_param, &sk, &TEST_ASSET, &(), &mut rng);
	let receiver_1 = receiver_1_full.prepared.process(&80, &mut rng);
	rng.fill_bytes(&mut sk);
	let receiver_2_full =
		MantaAssetFullReceiver::sample(&commit_param, &sk, &TEST_ASSET, &(), &mut rng);
	let receiver_2 = receiver_2_full.prepared.process(&130, &mut rng);

	// transfer circuit
	let transfer_circuit = TransferCircuit {
		// param
		commit_param,
		hash_param,

		// sender
		sender_1,
		sender_2,

		// receiver
		receiver_1,
		receiver_2,
	};

	let sanity_cs = ConstraintSystem::<Fq>::new_ref();
	transfer_circuit
		.clone()
		.generate_constraints(sanity_cs.clone())
		.unwrap();
	assert!(sanity_cs.is_satisfied().unwrap());

	// transfer pk_bytes
	let mut rng = ChaCha20Rng::from_seed(*rng_seed);
	let pk = generate_random_parameters::<Bls12_381, _, _>(transfer_circuit, &mut rng).unwrap();
	let mut transfer_pk_bytes: Vec<u8> = Vec::new();

	let mut vk_buf: Vec<u8> = vec![];
	let transfer_vk = &pk.vk;
	transfer_vk.serialize_uncompressed(&mut vk_buf).unwrap();
	#[cfg(features = "std")]
	println!("pk_uncompressed len {}", transfer_pk_bytes.len());
	println!("vk: {:?}", vk_buf);

	pk.serialize_uncompressed(&mut transfer_pk_bytes).unwrap();
	transfer_pk_bytes
}

// Generate ZKP keys for `reclaim` circuit.
#[cfg(feature = "std")]
fn manta_reclaim_zkp_key_gen(
	hash_param_seed: &[u8; 32],
	commit_param_seed: &[u8; 32],
	rng_seed: &[u8; 32],
) -> Vec<u8> {
	// rebuild the parameters from the inputs
	let mut rng = ChaCha20Rng::from_seed(*commit_param_seed);
	let commit_param = CommitmentScheme::setup(&mut rng).unwrap();

	let mut rng = ChaCha20Rng::from_seed(*hash_param_seed);
	let hash_param = Hash::setup(&mut rng).unwrap();

	let mut rng = ChaCha20Rng::from_seed(*rng_seed);
	let mut coins = Vec::new();
	let mut ledger = Vec::new();
	let mut sk = [0u8; 32];

	for e in 0..128 {
		rng.fill_bytes(&mut sk);

		let sender = MantaAsset::sample(&commit_param, &sk, &TEST_ASSET, &(e + 100), &mut rng);
		ledger.push(sender.commitment);
		coins.push(sender);
	}
	// sender's total value is 210
	let sender_1 = coins[0].clone();
	let sender_2 = coins[10].clone();

	let sender_1 = SenderMetaData::build(hash_param.clone(), sender_1, &ledger);
	let sender_2 = SenderMetaData::build(hash_param.clone(), sender_2, &ledger);

	// receiver's total value is also 210
	let receiver_full =
		MantaAssetFullReceiver::sample(&commit_param, &sk, &TEST_ASSET, &(), &mut rng);
	let receiver = receiver_full.prepared.process(&80, &mut rng);

	// transfer circuit
	let reclaim_circuit = ReclaimCircuit {
		// param
		commit_param,
		hash_param,

		// sender
		sender_1,
		sender_2,

		// receiver
		receiver,

		// reclaim value
		asset_id: AssetId::default(),
		reclaim_value: 130,
	};

	let sanity_cs = ConstraintSystem::<Fq>::new_ref();
	reclaim_circuit
		.clone()
		.generate_constraints(sanity_cs.clone())
		.unwrap();
	assert!(sanity_cs.is_satisfied().unwrap());

	// reclaim pk_bytes
	let mut rng = ChaCha20Rng::from_seed(*rng_seed);
	let pk = generate_random_parameters::<Bls12_381, _, _>(reclaim_circuit, &mut rng).unwrap();
	let mut reclaim_pk_bytes: Vec<u8> = Vec::new();

	let mut vk_buf: Vec<u8> = vec![];
	let reclaim_vk = &pk.vk;
	reclaim_vk.serialize_uncompressed(&mut vk_buf).unwrap();
	println!("pk_uncompressed len {}", reclaim_pk_bytes.len());
	println!("vk: {:?}", vk_buf);

	pk.serialize_uncompressed(&mut reclaim_pk_bytes).unwrap();
	reclaim_pk_bytes
}

/// Pre-computed,
/// serialized verification key transfer proof.
const TRANSFER_VKBYTES: [u8; 2312] = [
	235, 53, 181, 140, 33, 225, 93, 94, 112, 108, 149, 110, 173, 80, 36, 124, 99, 113, 4, 251, 191,
	234, 245, 178, 48, 38, 237, 44, 14, 238, 20, 140, 222, 192, 225, 59, 179, 14, 135, 68, 121, 26,
	216, 223, 154, 197, 222, 24, 123, 223, 192, 56, 109, 201, 61, 22, 107, 35, 13, 69, 62, 216,
	178, 221, 130, 107, 27, 189, 165, 30, 111, 163, 232, 240, 135, 195, 105, 188, 169, 213, 65,
	119, 198, 134, 149, 107, 12, 161, 121, 42, 245, 75, 27, 70, 72, 5, 170, 17, 199, 220, 89, 180,
	73, 186, 11, 242, 241, 22, 202, 57, 200, 3, 241, 222, 203, 149, 197, 87, 170, 4, 129, 229, 144,
	61, 172, 115, 109, 39, 52, 146, 175, 194, 167, 148, 222, 20, 186, 27, 65, 105, 60, 252, 139, 8,
	51, 175, 48, 78, 171, 104, 197, 86, 147, 27, 229, 250, 204, 136, 53, 110, 98, 234, 182, 44,
	244, 35, 167, 17, 43, 43, 170, 100, 150, 207, 224, 15, 1, 28, 54, 238, 42, 169, 77, 104, 203,
	179, 41, 167, 0, 202, 148, 10, 42, 110, 23, 45, 82, 1, 109, 103, 23, 215, 183, 232, 133, 39,
	215, 131, 177, 49, 56, 35, 76, 94, 63, 163, 4, 245, 63, 193, 147, 79, 130, 138, 229, 215, 0,
	55, 95, 244, 158, 56, 205, 250, 149, 147, 59, 96, 123, 13, 48, 1, 127, 169, 85, 86, 91, 83,
	187, 27, 152, 12, 115, 17, 90, 116, 208, 153, 2, 73, 22, 147, 3, 255, 19, 253, 93, 124, 195, 1,
	83, 215, 81, 249, 233, 190, 130, 164, 187, 139, 120, 220, 128, 213, 7, 17, 71, 20, 96, 207, 10,
	40, 207, 66, 178, 17, 70, 197, 218, 196, 24, 98, 233, 206, 134, 230, 17, 28, 76, 68, 123, 173,
	255, 185, 222, 155, 141, 130, 230, 97, 243, 247, 17, 195, 166, 169, 60, 111, 117, 101, 14, 165,
	98, 187, 13, 0, 241, 69, 10, 254, 0, 221, 20, 33, 125, 119, 29, 222, 59, 239, 59, 163, 56, 105,
	100, 148, 127, 185, 216, 242, 55, 244, 124, 63, 126, 33, 67, 141, 129, 14, 138, 13, 44, 41, 44,
	224, 53, 109, 217, 213, 120, 75, 79, 7, 3, 124, 76, 178, 220, 197, 40, 108, 108, 9, 140, 141,
	94, 24, 15, 65, 47, 69, 177, 36, 76, 93, 162, 248, 44, 85, 239, 40, 148, 131, 0, 26, 158, 215,
	225, 197, 97, 132, 36, 246, 125, 187, 170, 156, 206, 193, 51, 10, 133, 66, 28, 208, 1, 118, 94,
	31, 67, 89, 54, 185, 216, 40, 105, 181, 250, 196, 52, 197, 6, 117, 96, 242, 250, 145, 46, 102,
	106, 117, 138, 144, 133, 21, 221, 58, 84, 35, 169, 120, 75, 62, 247, 243, 216, 125, 116, 15,
	176, 55, 166, 130, 215, 223, 142, 17, 137, 174, 11, 54, 244, 94, 100, 40, 181, 138, 16, 203,
	140, 115, 25, 11, 106, 72, 64, 213, 245, 237, 63, 61, 149, 232, 13, 189, 131, 20, 118, 253, 93,
	199, 11, 134, 127, 71, 38, 0, 116, 202, 63, 233, 163, 91, 1, 160, 216, 204, 228, 99, 89, 31,
	167, 75, 177, 101, 99, 26, 159, 69, 100, 182, 8, 124, 130, 69, 165, 200, 217, 161, 60, 83, 152,
	198, 232, 84, 197, 79, 33, 137, 215, 41, 235, 209, 51, 10, 171, 74, 119, 143, 200, 82, 137,
	249, 168, 166, 46, 116, 103, 73, 219, 176, 151, 194, 51, 68, 71, 225, 144, 194, 125, 144, 238,
	106, 204, 60, 61, 85, 12, 223, 78, 11, 13, 185, 28, 95, 235, 36, 40, 128, 196, 136, 149, 2,
	187, 123, 249, 184, 177, 73, 133, 209, 193, 180, 199, 3, 4, 47, 191, 155, 220, 56, 142, 137,
	117, 107, 197, 212, 222, 118, 94, 2, 195, 33, 105, 94, 101, 135, 22, 215, 144, 100, 149, 127,
	169, 221, 97, 21, 255, 27, 43, 24, 17, 0, 0, 0, 0, 0, 0, 0, 133, 202, 106, 12, 59, 213, 99,
	140, 177, 17, 100, 182, 110, 212, 165, 210, 131, 251, 173, 191, 28, 8, 208, 43, 53, 78, 128,
	249, 227, 0, 202, 252, 89, 53, 49, 30, 12, 179, 57, 241, 8, 102, 112, 214, 171, 177, 79, 21,
	150, 189, 206, 105, 101, 108, 184, 23, 248, 96, 99, 40, 228, 30, 16, 158, 2, 205, 24, 37, 208,
	49, 14, 251, 199, 240, 26, 112, 100, 230, 232, 166, 9, 50, 229, 113, 75, 26, 94, 240, 184, 206,
	179, 253, 121, 162, 99, 9, 231, 226, 218, 44, 224, 200, 104, 5, 53, 160, 2, 61, 246, 77, 113,
	233, 61, 239, 112, 34, 153, 234, 221, 239, 64, 236, 173, 253, 22, 168, 6, 199, 32, 103, 174,
	97, 149, 82, 187, 47, 141, 196, 56, 15, 61, 175, 46, 13, 187, 197, 65, 160, 46, 45, 19, 152,
	102, 189, 113, 197, 174, 31, 49, 87, 99, 152, 134, 0, 84, 138, 124, 124, 255, 37, 235, 54, 165,
	223, 188, 90, 164, 234, 237, 177, 207, 145, 167, 179, 198, 38, 88, 32, 76, 157, 134, 22, 82,
	55, 149, 143, 245, 59, 94, 37, 111, 29, 220, 153, 179, 121, 118, 131, 0, 96, 107, 247, 218,
	231, 57, 235, 186, 252, 123, 182, 75, 200, 234, 192, 44, 204, 66, 255, 62, 31, 3, 150, 219,
	241, 6, 141, 90, 227, 210, 21, 39, 73, 225, 88, 121, 185, 56, 184, 190, 212, 158, 226, 189,
	218, 155, 34, 223, 175, 167, 108, 112, 53, 105, 219, 203, 123, 76, 123, 186, 193, 178, 154,
	115, 220, 248, 235, 111, 51, 215, 77, 17, 114, 58, 224, 71, 121, 122, 23, 129, 183, 251, 219,
	209, 71, 31, 234, 42, 102, 71, 165, 66, 72, 173, 242, 75, 49, 247, 182, 59, 211, 154, 225, 191,
	216, 142, 149, 48, 138, 11, 121, 68, 249, 121, 200, 92, 34, 137, 90, 207, 170, 143, 2, 13, 99,
	216, 7, 237, 105, 43, 70, 76, 111, 172, 56, 30, 169, 249, 173, 143, 211, 139, 166, 103, 90,
	221, 25, 50, 76, 68, 237, 61, 119, 157, 93, 213, 212, 183, 158, 31, 194, 82, 211, 227, 227, 22,
	124, 196, 28, 70, 250, 7, 36, 0, 25, 117, 9, 90, 149, 148, 95, 196, 111, 135, 94, 71, 180, 173,
	47, 127, 235, 252, 60, 181, 248, 99, 67, 83, 27, 238, 232, 242, 206, 180, 199, 117, 88, 120,
	238, 34, 215, 67, 136, 246, 170, 189, 76, 19, 234, 153, 12, 66, 10, 224, 165, 137, 244, 91,
	209, 217, 149, 74, 65, 106, 176, 11, 132, 8, 40, 129, 89, 26, 182, 108, 195, 247, 202, 47, 46,
	104, 162, 183, 93, 157, 181, 196, 154, 143, 37, 193, 155, 51, 64, 106, 148, 166, 47, 189, 201,
	194, 0, 15, 245, 165, 142, 190, 127, 225, 231, 86, 165, 169, 94, 141, 23, 218, 63, 197, 166,
	213, 205, 50, 52, 184, 183, 226, 145, 169, 161, 178, 31, 123, 193, 107, 81, 214, 36, 152, 213,
	20, 9, 143, 84, 68, 118, 22, 61, 28, 21, 177, 152, 201, 30, 163, 65, 182, 192, 57, 150, 10,
	209, 28, 226, 111, 4, 181, 73, 102, 201, 106, 248, 185, 138, 238, 126, 46, 206, 88, 236, 90,
	129, 75, 41, 27, 154, 151, 138, 146, 106, 217, 22, 121, 245, 156, 190, 45, 16, 66, 29, 43, 48,
	25, 235, 28, 83, 98, 172, 232, 79, 64, 96, 199, 194, 156, 87, 198, 23, 105, 91, 116, 141, 41,
	28, 122, 48, 130, 40, 113, 94, 5, 130, 137, 79, 147, 58, 67, 197, 87, 238, 114, 121, 133, 99,
	6, 12, 213, 204, 83, 122, 7, 248, 61, 159, 17, 207, 44, 68, 22, 26, 159, 60, 146, 45, 177, 91,
	16, 17, 135, 220, 37, 176, 195, 115, 235, 163, 15, 67, 61, 238, 26, 160, 209, 145, 87, 162,
	167, 52, 37, 221, 139, 68, 122, 9, 62, 250, 240, 172, 223, 67, 254, 141, 237, 18, 169, 184,
	240, 69, 60, 236, 140, 52, 66, 231, 209, 8, 203, 217, 158, 252, 79, 105, 172, 147, 5, 248, 8,
	108, 36, 34, 188, 35, 194, 204, 82, 120, 0, 77, 139, 33, 254, 25, 203, 120, 202, 15, 5, 109,
	74, 166, 213, 133, 172, 98, 130, 234, 185, 244, 28, 222, 128, 220, 181, 142, 109, 58, 193, 253,
	166, 25, 199, 145, 231, 143, 215, 92, 106, 166, 122, 118, 149, 76, 41, 158, 235, 119, 249, 223,
	47, 23, 99, 82, 237, 26, 30, 46, 228, 101, 179, 226, 140, 237, 159, 9, 14, 229, 75, 204, 161,
	231, 124, 144, 88, 120, 183, 106, 248, 56, 82, 25, 42, 245, 111, 201, 100, 4, 47, 194, 142, 2,
	58, 95, 159, 190, 38, 243, 180, 10, 243, 163, 143, 125, 167, 205, 115, 196, 222, 161, 44, 187,
	196, 161, 50, 57, 194, 235, 195, 51, 98, 52, 10, 2, 251, 80, 157, 38, 176, 233, 138, 129, 86,
	251, 145, 43, 132, 139, 111, 53, 110, 134, 108, 64, 224, 29, 58, 16, 148, 214, 176, 10, 59,
	185, 68, 167, 71, 191, 91, 168, 63, 122, 61, 130, 191, 252, 236, 187, 237, 70, 118, 86, 213,
	28, 91, 42, 80, 133, 10, 12, 200, 40, 67, 102, 185, 111, 3, 153, 148, 161, 6, 72, 30, 219, 91,
	16, 203, 108, 204, 88, 185, 249, 94, 76, 68, 154, 239, 184, 27, 95, 176, 136, 171, 3, 235, 183,
	32, 158, 23, 151, 60, 240, 127, 164, 99, 218, 213, 231, 94, 69, 84, 166, 203, 118, 112, 183,
	247, 42, 137, 192, 62, 213, 191, 24, 173, 26, 35, 242, 190, 98, 106, 232, 174, 135, 194, 110,
	170, 90, 253, 20, 252, 240, 47, 185, 172, 178, 164, 146, 144, 160, 180, 217, 239, 35, 201, 153,
	235, 228, 117, 64, 116, 243, 231, 88, 38, 35, 103, 24, 36, 101, 13, 16, 65, 238, 218, 180, 96,
	139, 246, 181, 247, 205, 120, 13, 83, 255, 93, 93, 125, 77, 121, 247, 196, 216, 212, 102, 161,
	148, 74, 205, 165, 4, 159, 171, 165, 89, 20, 175, 75, 74, 49, 102, 253, 27, 131, 249, 185, 98,
	226, 21, 53, 177, 28, 65, 33, 55, 209, 70, 206, 155, 220, 113, 197, 7, 39, 177, 26, 67, 211,
	18, 57, 4, 113, 70, 203, 83, 7, 255, 49, 122, 101, 12, 80, 139, 90, 216, 131, 207, 96, 144,
	129, 250, 168, 216, 226, 237, 75, 13, 78, 197, 248, 162, 0, 7, 92, 218, 34, 142, 196, 202, 125,
	17, 106, 211, 247, 123, 105, 76, 77, 21, 48, 104, 100, 12, 130, 58, 189, 213, 199, 193, 163,
	175, 209, 168, 68, 171, 81, 9, 182, 53, 125, 57, 253, 124, 212, 6, 77, 171, 7, 84, 55, 208, 38,
	244, 238, 237, 211, 29, 161, 112, 202, 28, 192, 79, 220, 21, 205, 214, 58, 31, 243, 52, 57,
	221, 147, 96, 57, 20, 159, 199, 81, 194, 20, 168, 128, 23, 79, 215, 183, 180, 226, 159, 216,
	15, 32, 92, 38, 4, 95, 104, 17, 53, 163, 154, 193, 232, 57, 204, 50, 215, 216, 38, 188, 112,
	240, 44, 35, 5, 30, 51, 197, 195, 121, 13, 168, 92, 248, 25, 84, 72, 117, 164, 152, 106, 249,
	236, 167, 252, 94, 219, 107, 25, 224, 107, 28, 137, 13, 134, 152, 243, 184, 203, 178, 235, 133,
	224, 206, 108, 73, 179, 35, 173, 170, 122, 39, 48, 179, 31, 82, 252, 253, 187, 75, 248, 23, 13,
	202, 16, 2, 52, 113, 5, 196, 109, 48, 90, 206, 243, 237, 8, 55, 28, 77, 97, 112, 66, 37, 121,
	97, 11, 90, 204, 130, 94, 223, 122, 52, 122, 135, 8, 53, 141, 213, 147, 234, 208, 159, 87, 131,
	60, 189, 161, 124, 134, 127, 76, 241, 110, 140, 83, 152, 0, 36, 34, 195, 81, 141, 5, 96, 195,
	167, 19, 242, 109, 237, 186, 14, 48, 64, 96, 3, 2, 240, 61, 229, 182, 170, 184, 171, 101, 236,
	52, 237, 33, 244, 190, 152, 216, 213, 112, 69, 132, 189, 32, 40, 90, 41, 66, 115, 149, 53, 137,
	21, 104, 5, 5, 212, 113, 222, 36, 23, 90, 91, 55, 65, 249, 10, 137, 241, 4, 133, 110, 149, 178,
	79, 137, 64, 165, 85, 183, 42, 206, 4, 225, 100, 25, 222, 30, 141, 226, 54, 115, 160, 224, 146,
	244, 187, 209, 11, 165, 35, 122, 204, 2, 177, 231, 75, 119, 232, 77, 57, 35, 11, 228, 121, 225,
	81, 248, 77, 52, 216, 54, 81, 225, 13, 91, 254, 64, 91, 36, 99, 189, 144, 79, 17, 149, 125, 91,
	208, 241, 200, 106, 234, 120, 93, 25, 143, 89, 87, 113, 23, 11, 150, 252, 64, 64, 242, 203, 63,
	178, 248, 82, 123, 165, 216, 200, 56, 185, 114, 215, 34, 33, 153, 64, 229, 239, 72, 107, 2,
	132, 150, 190, 69, 233, 132, 20, 171, 11, 149, 41, 176, 15, 204, 157, 67, 90, 103, 64, 32, 0,
	105, 88, 44, 6, 163, 160, 80, 228, 24, 239, 91, 247, 202, 28, 2, 147, 55, 226, 86, 171, 156,
	133, 188, 48, 9, 229, 248, 32, 155, 208, 202, 5, 252, 149, 135, 243, 14, 215, 32, 42, 19, 16,
	118, 250, 225, 144, 87, 14, 222, 147, 47, 182, 170, 112, 252, 198, 176, 58, 177, 208, 251, 63,
	237, 236, 180, 40, 74, 81, 57, 46, 124, 170, 140, 107, 228, 23, 7, 101, 195, 104, 214, 71, 35,
	97, 250, 215, 124, 164, 163, 55, 82, 179, 216, 193, 157, 6,
];

/// Pre-computed,
/// serialized verification key reclaim proof.
const RECLAIM_VKBYTES: [u8; 2312] = [
	235, 53, 181, 140, 33, 225, 93, 94, 112, 108, 149, 110, 173, 80, 36, 124, 99, 113, 4, 251, 191,
	234, 245, 178, 48, 38, 237, 44, 14, 238, 20, 140, 222, 192, 225, 59, 179, 14, 135, 68, 121, 26,
	216, 223, 154, 197, 222, 24, 123, 223, 192, 56, 109, 201, 61, 22, 107, 35, 13, 69, 62, 216,
	178, 221, 130, 107, 27, 189, 165, 30, 111, 163, 232, 240, 135, 195, 105, 188, 169, 213, 65,
	119, 198, 134, 149, 107, 12, 161, 121, 42, 245, 75, 27, 70, 72, 5, 170, 17, 199, 220, 89, 180,
	73, 186, 11, 242, 241, 22, 202, 57, 200, 3, 241, 222, 203, 149, 197, 87, 170, 4, 129, 229, 144,
	61, 172, 115, 109, 39, 52, 146, 175, 194, 167, 148, 222, 20, 186, 27, 65, 105, 60, 252, 139, 8,
	51, 175, 48, 78, 171, 104, 197, 86, 147, 27, 229, 250, 204, 136, 53, 110, 98, 234, 182, 44,
	244, 35, 167, 17, 43, 43, 170, 100, 150, 207, 224, 15, 1, 28, 54, 238, 42, 169, 77, 104, 203,
	179, 41, 167, 0, 202, 148, 10, 42, 110, 23, 45, 82, 1, 109, 103, 23, 215, 183, 232, 133, 39,
	215, 131, 177, 49, 56, 35, 76, 94, 63, 163, 4, 245, 63, 193, 147, 79, 130, 138, 229, 215, 0,
	55, 95, 244, 158, 56, 205, 250, 149, 147, 59, 96, 123, 13, 48, 1, 127, 169, 85, 86, 91, 83,
	187, 27, 152, 12, 115, 17, 90, 116, 208, 153, 2, 73, 22, 147, 3, 255, 19, 253, 93, 124, 195, 1,
	83, 215, 81, 249, 233, 190, 130, 164, 187, 139, 120, 220, 128, 213, 7, 17, 71, 20, 96, 207, 10,
	40, 207, 66, 178, 17, 70, 197, 218, 196, 24, 98, 233, 206, 134, 230, 17, 28, 76, 68, 123, 173,
	255, 185, 222, 155, 141, 130, 230, 97, 243, 247, 17, 195, 166, 169, 60, 111, 117, 101, 14, 165,
	98, 187, 13, 0, 241, 69, 10, 254, 0, 221, 20, 33, 125, 119, 29, 222, 59, 239, 59, 163, 56, 105,
	100, 148, 127, 185, 216, 242, 55, 244, 124, 63, 126, 33, 67, 141, 129, 14, 138, 13, 44, 41, 44,
	224, 53, 109, 217, 213, 120, 75, 79, 7, 3, 124, 76, 178, 220, 197, 40, 108, 108, 9, 140, 141,
	94, 24, 15, 65, 47, 69, 177, 36, 76, 93, 162, 248, 44, 85, 239, 40, 148, 131, 0, 26, 158, 215,
	225, 197, 97, 132, 36, 246, 125, 187, 170, 156, 206, 193, 51, 10, 133, 66, 28, 208, 1, 118, 94,
	31, 67, 89, 54, 185, 216, 40, 105, 181, 250, 196, 52, 197, 6, 117, 96, 242, 250, 145, 46, 102,
	106, 117, 138, 144, 133, 21, 221, 58, 84, 35, 169, 120, 75, 62, 247, 243, 216, 125, 116, 15,
	176, 55, 166, 130, 215, 223, 142, 17, 137, 174, 11, 54, 244, 94, 100, 40, 181, 138, 16, 203,
	140, 115, 25, 11, 106, 72, 64, 213, 245, 237, 63, 61, 149, 232, 13, 189, 131, 20, 118, 253, 93,
	199, 11, 134, 127, 71, 38, 0, 116, 202, 63, 233, 163, 91, 1, 160, 216, 204, 228, 99, 89, 31,
	167, 75, 177, 101, 99, 26, 159, 69, 100, 182, 8, 124, 130, 69, 165, 200, 217, 161, 60, 83, 152,
	198, 232, 84, 197, 79, 33, 137, 215, 41, 235, 209, 51, 10, 171, 74, 119, 143, 200, 82, 137,
	249, 168, 166, 46, 116, 103, 73, 219, 176, 151, 194, 51, 68, 71, 225, 144, 194, 125, 144, 238,
	106, 204, 60, 61, 85, 12, 223, 78, 11, 13, 185, 28, 95, 235, 36, 40, 128, 196, 136, 149, 2,
	187, 123, 249, 184, 177, 73, 133, 209, 193, 180, 199, 3, 4, 47, 191, 155, 220, 56, 142, 137,
	117, 107, 197, 212, 222, 118, 94, 2, 195, 33, 105, 94, 101, 135, 22, 215, 144, 100, 149, 127,
	169, 221, 97, 21, 255, 27, 43, 24, 17, 0, 0, 0, 0, 0, 0, 0, 107, 47, 29, 243, 58, 199, 42, 41,
	214, 206, 232, 96, 73, 247, 185, 206, 208, 232, 68, 114, 46, 89, 118, 145, 81, 207, 188, 13,
	106, 22, 220, 195, 19, 23, 80, 234, 129, 117, 39, 11, 167, 85, 111, 118, 163, 224, 30, 16, 124,
	41, 84, 115, 56, 82, 133, 76, 73, 143, 31, 108, 93, 40, 101, 234, 230, 11, 175, 88, 185, 103,
	17, 133, 83, 159, 44, 90, 155, 251, 198, 140, 171, 237, 205, 144, 157, 116, 203, 119, 151, 240,
	157, 236, 244, 70, 254, 5, 8, 150, 220, 170, 188, 41, 141, 22, 206, 213, 83, 111, 255, 226,
	111, 193, 131, 198, 146, 143, 55, 167, 134, 160, 22, 15, 185, 21, 52, 250, 110, 23, 211, 45,
	148, 5, 4, 142, 150, 50, 4, 21, 203, 205, 154, 202, 239, 23, 17, 205, 247, 115, 103, 133, 233,
	81, 217, 193, 215, 14, 215, 7, 218, 184, 239, 105, 51, 155, 124, 87, 72, 62, 143, 83, 109, 12,
	8, 135, 178, 254, 69, 81, 158, 151, 27, 31, 12, 234, 88, 160, 124, 213, 205, 203, 49, 6, 143,
	158, 40, 122, 28, 10, 186, 82, 101, 52, 207, 134, 144, 83, 214, 247, 51, 101, 90, 169, 137, 3,
	232, 110, 70, 212, 49, 204, 159, 249, 81, 111, 241, 242, 110, 158, 55, 182, 76, 223, 35, 18,
	75, 199, 35, 203, 214, 1, 199, 126, 248, 73, 180, 150, 68, 84, 148, 165, 254, 45, 218, 85, 119,
	42, 44, 84, 173, 234, 44, 53, 27, 162, 28, 245, 45, 124, 49, 62, 52, 74, 54, 229, 80, 148, 197,
	205, 67, 17, 201, 206, 237, 59, 94, 180, 137, 23, 41, 133, 158, 109, 200, 152, 45, 103, 71,
	129, 1, 201, 244, 92, 143, 187, 82, 151, 18, 74, 42, 214, 87, 177, 248, 222, 221, 156, 102,
	209, 148, 202, 85, 25, 94, 191, 170, 195, 11, 147, 211, 154, 4, 230, 4, 120, 249, 6, 209, 35,
	249, 179, 25, 31, 103, 233, 49, 205, 141, 8, 106, 92, 60, 109, 33, 148, 99, 146, 197, 107, 36,
	182, 159, 239, 241, 18, 30, 35, 169, 9, 34, 118, 149, 155, 236, 216, 43, 186, 87, 27, 68, 183,
	169, 57, 29, 21, 147, 2, 149, 199, 77, 192, 109, 52, 194, 183, 61, 220, 202, 150, 108, 239,
	154, 82, 93, 36, 181, 136, 230, 255, 158, 21, 222, 18, 108, 103, 150, 213, 241, 23, 115, 118,
	26, 164, 6, 223, 80, 254, 180, 250, 240, 73, 57, 0, 235, 14, 37, 88, 163, 72, 101, 0, 98, 195,
	167, 164, 232, 80, 75, 202, 193, 17, 116, 128, 84, 199, 187, 152, 29, 189, 110, 72, 120, 13,
	221, 105, 28, 229, 150, 189, 5, 149, 233, 66, 237, 85, 186, 253, 115, 131, 209, 24, 57, 172,
	21, 12, 175, 168, 226, 57, 89, 176, 14, 4, 182, 152, 240, 139, 151, 123, 222, 200, 106, 47, 53,
	232, 108, 29, 101, 172, 206, 226, 203, 19, 249, 192, 127, 240, 87, 3, 220, 21, 100, 19, 67, 23,
	141, 95, 74, 0, 138, 243, 228, 252, 90, 131, 76, 47, 19, 81, 80, 7, 221, 202, 120, 102, 250,
	253, 133, 105, 28, 34, 97, 21, 64, 175, 225, 29, 239, 50, 240, 13, 49, 190, 100, 206, 250, 95,
	98, 72, 112, 32, 71, 244, 3, 200, 220, 6, 206, 59, 94, 68, 152, 171, 3, 197, 18, 60, 27, 239,
	172, 83, 120, 197, 248, 120, 140, 208, 91, 196, 34, 159, 54, 71, 208, 207, 120, 47, 36, 37,
	217, 72, 4, 5, 63, 58, 132, 110, 195, 254, 155, 74, 60, 132, 213, 0, 222, 187, 165, 25, 133,
	238, 181, 36, 126, 161, 2, 225, 110, 193, 148, 141, 236, 202, 122, 149, 72, 57, 59, 207, 189,
	81, 191, 80, 231, 152, 204, 95, 153, 116, 96, 241, 50, 200, 210, 178, 49, 183, 51, 47, 93, 249,
	123, 17, 184, 251, 134, 83, 15, 121, 227, 192, 3, 47, 30, 99, 42, 74, 86, 117, 25, 196, 133,
	45, 182, 236, 205, 223, 101, 75, 59, 170, 231, 220, 240, 39, 24, 96, 242, 129, 89, 175, 0, 153,
	83, 142, 201, 225, 240, 24, 74, 9, 204, 204, 153, 100, 17, 137, 99, 98, 29, 74, 113, 125, 102,
	186, 100, 3, 107, 45, 199, 178, 11, 13, 136, 163, 208, 159, 115, 176, 54, 211, 82, 94, 253,
	186, 10, 197, 221, 12, 203, 47, 185, 233, 77, 222, 159, 135, 226, 11, 159, 154, 197, 85, 55,
	169, 118, 133, 96, 123, 101, 179, 99, 235, 167, 80, 82, 180, 122, 153, 247, 97, 57, 181, 243,
	26, 8, 229, 109, 159, 133, 249, 78, 3, 43, 77, 234, 231, 15, 52, 162, 58, 40, 92, 155, 69, 120,
	19, 66, 240, 152, 136, 6, 131, 234, 185, 75, 91, 205, 255, 69, 103, 162, 167, 27, 226, 196,
	154, 203, 198, 50, 213, 123, 172, 0, 32, 250, 22, 151, 155, 15, 66, 227, 8, 142, 250, 64, 177,
	59, 159, 96, 138, 153, 178, 198, 12, 137, 25, 171, 71, 23, 116, 52, 221, 185, 31, 38, 129, 3,
	207, 142, 21, 217, 244, 26, 225, 242, 238, 125, 137, 116, 55, 43, 235, 238, 196, 187, 199, 226,
	139, 158, 111, 220, 90, 149, 182, 202, 210, 221, 99, 75, 24, 199, 6, 89, 31, 0, 46, 47, 14,
	195, 119, 1, 223, 12, 96, 145, 5, 184, 42, 165, 82, 224, 211, 145, 254, 15, 188, 155, 227, 187,
	48, 104, 134, 27, 0, 45, 8, 116, 199, 68, 161, 29, 79, 220, 125, 160, 134, 24, 29, 171, 10,
	240, 211, 217, 153, 95, 19, 5, 84, 92, 99, 181, 119, 10, 235, 183, 24, 49, 182, 12, 220, 185,
	251, 179, 68, 178, 139, 114, 181, 178, 244, 165, 218, 0, 178, 194, 114, 15, 96, 203, 41, 60,
	95, 55, 188, 152, 124, 67, 11, 117, 123, 23, 89, 49, 223, 167, 171, 117, 137, 136, 134, 169,
	199, 91, 237, 217, 23, 247, 76, 158, 68, 39, 10, 114, 134, 106, 75, 77, 35, 241, 192, 36, 72,
	117, 218, 131, 221, 217, 45, 221, 150, 227, 82, 119, 131, 77, 7, 244, 237, 150, 181, 245, 111,
	118, 37, 98, 195, 203, 100, 35, 70, 102, 181, 94, 17, 182, 9, 57, 142, 60, 202, 14, 156, 54,
	233, 221, 77, 182, 97, 109, 249, 45, 127, 19, 243, 238, 34, 114, 160, 198, 244, 233, 218, 60,
	24, 158, 28, 188, 254, 62, 247, 130, 103, 122, 156, 4, 143, 98, 224, 211, 99, 81, 84, 187, 80,
	212, 132, 215, 26, 96, 217, 73, 7, 179, 114, 158, 70, 57, 130, 134, 221, 215, 111, 96, 39, 137,
	220, 195, 91, 115, 255, 226, 4, 162, 10, 55, 143, 250, 176, 23, 103, 44, 217, 196, 70, 195, 75,
	215, 128, 112, 252, 181, 159, 55, 109, 129, 201, 249, 74, 22, 32, 183, 45, 46, 162, 149, 106,
	26, 77, 34, 248, 230, 144, 183, 221, 46, 151, 171, 26, 110, 10, 53, 247, 128, 140, 167, 10,
	241, 108, 160, 209, 237, 44, 120, 74, 16, 20, 132, 103, 253, 104, 216, 118, 118, 76, 179, 178,
	232, 115, 43, 14, 35, 57, 238, 95, 45, 133, 127, 247, 232, 150, 238, 237, 152, 45, 37, 84, 86,
	1, 141, 182, 112, 41, 35, 253, 50, 53, 155, 48, 246, 250, 184, 250, 236, 19, 103, 172, 231,
	135, 87, 111, 150, 10, 188, 54, 230, 175, 3, 243, 57, 72, 148, 149, 156, 67, 114, 9, 223, 216,
	41, 17, 127, 74, 21, 48, 158, 6, 45, 2, 242, 92, 243, 208, 202, 176, 122, 145, 74, 152, 235,
	35, 33, 52, 71, 106, 13, 240, 195, 142, 53, 54, 163, 167, 31, 10, 97, 222, 67, 72, 246, 85,
	203, 223, 60, 205, 226, 73, 237, 9, 217, 116, 253, 204, 140, 5, 247, 233, 222, 198, 2, 11, 169,
	70, 160, 202, 58, 224, 188, 24, 131, 187, 117, 217, 107, 34, 240, 129, 166, 92, 96, 24, 81,
	134, 177, 28, 208, 242, 6, 222, 208, 163, 134, 213, 97, 66, 188, 154, 42, 178, 224, 24, 104,
	12, 117, 162, 81, 172, 97, 210, 175, 183, 169, 212, 80, 92, 78, 195, 161, 106, 227, 218, 214,
	179, 174, 28, 161, 149, 169, 203, 35, 164, 253, 226, 85, 59, 211, 1, 185, 101, 234, 241, 246,
	120, 116, 80, 121, 15, 252, 95, 140, 8, 67, 248, 228, 67, 224, 56, 198, 169, 190, 92, 220, 223,
	44, 117, 117, 159, 186, 145, 109, 47, 96, 23, 203, 24, 191, 153, 103, 157, 181, 231, 111, 180,
	30, 82, 215, 122, 17, 139, 38, 149, 185, 41, 91, 26, 40, 4, 195, 15, 149, 8, 88, 193, 68, 11,
	6, 79, 236, 1, 50, 29, 239, 40, 58, 97, 201, 244, 62, 126, 106, 117, 158, 213, 246, 144, 11,
	214, 201, 156, 87, 220, 24, 26, 58, 193, 137, 90, 159, 236, 178, 38, 154, 196, 114, 49, 25, 12,
	43, 63, 187, 47, 199, 159, 208, 231, 197, 120, 21, 116, 169, 117, 239, 141, 47, 252, 20, 38,
	134, 191, 164, 58, 157, 46, 250, 200, 45, 210, 166, 52, 4, 149, 226, 56, 56, 187, 230, 107,
	183, 103, 60, 72, 29, 56, 90, 2, 171, 30, 240, 59, 169, 253, 94, 112, 206, 95, 75, 119, 11, 35,
	180, 114, 96, 169, 128, 102, 80, 43, 219, 155, 12, 88, 28, 70, 53, 215, 169, 70, 202, 125, 44,
	226, 98, 218, 109, 11, 175, 49, 147, 165, 159, 227, 233, 23,
];
