package main

import (
	"bytes"
	"io"
	"path/filepath"
	"testing"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/groth16"
	mpc "github.com/consensys/gnark/backend/groth16/bn254/mpcsetup"
	cs "github.com/consensys/gnark/constraint/bn254"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
)

type bytesWriter []byte

func (value bytesWriter) WriteTo(writer io.Writer) (int64, error) {
	written, err := writer.Write(value)
	return int64(written), err
}

func TestParseBeaconRequiresCanonicalPublicEntropy(t *testing.T) {
	valid := "0x" + string(bytes.Repeat([]byte{'a'}, 64))
	value, canonical, err := parseBeacon(valid)
	if err != nil || canonical != valid || len(value) != 32 {
		t.Fatalf("valid beacon rejected: %v", err)
	}
	for _, invalid := range []string{"", "0x01", "0x" + string(bytes.Repeat([]byte{'A'}, 64))} {
		if _, _, err := parseBeacon(invalid); err == nil {
			t.Fatalf("invalid beacon accepted: %q", invalid)
		}
	}
}

func TestContributionOutputCannotBeOverwritten(t *testing.T) {
	path := filepath.Join(t.TempDir(), "contribution.bin")
	value := bytesWriter("first")
	if err := writeToExclusive(path, value); err != nil {
		t.Fatal(err)
	}
	if err := writeToExclusive(path, bytesWriter("second")); err == nil {
		t.Fatal("existing contribution was overwritten")
	}
}

func TestContributionInventoryRejectsAmbiguity(t *testing.T) {
	if _, err := splitPaths(""); err == nil {
		t.Fatal("empty contribution inventory accepted")
	}
	if _, err := splitPaths("a, b"); err == nil {
		t.Fatal("whitespace-ambiguous inventory accepted")
	}
	paths, err := splitPaths("a,b")
	if err != nil || len(paths) != 2 {
		t.Fatalf("valid inventory rejected: %v", err)
	}
}

func TestGroth16SolidityUsesTheSP1WrapperABI(t *testing.T) {
	input := "pragma solidity ^0.8.0;\ncontract Verifier {\nfunction verifyProof() public {}\n}"
	output, err := normalizeGroth16Solidity(input)
	if err != nil {
		t.Fatal(err)
	}
	for _, expected := range []string{
		"pragma solidity ^0.8.20;",
		"contract Groth16Verifier {",
		"function Verify(",
	} {
		if !bytes.Contains([]byte(output), []byte(expected)) {
			t.Fatalf("normalized verifier is missing %q", expected)
		}
	}
	if _, err := normalizeGroth16Solidity(output); err == nil {
		t.Fatal("already-normalized verifier was accepted as raw gnark output")
	}
}

type squareCircuit struct {
	Square frontend.Variable `gnark:",public"`
	Root   frontend.Variable
}

func (circuit *squareCircuit) Define(api frontend.API) error {
	api.AssertIsEqual(api.Mul(circuit.Root, circuit.Root), circuit.Square)
	return nil
}

func TestMpcRoundTripAndSp1DumpEncoding(t *testing.T) {
	snapshot := func(source io.WriterTo, destination io.ReaderFrom) {
		buffer := new(bytes.Buffer)
		if _, err := source.WriteTo(buffer); err != nil {
			t.Fatal(err)
		}
		if _, err := destination.ReadFrom(buffer); err != nil {
			t.Fatal(err)
		}
	}
	compiled, err := frontend.Compile(
		ecc.BN254.ScalarField(), r1cs.NewBuilder, &squareCircuit{},
	)
	if err != nil {
		t.Fatal(err)
	}
	typed, ok := compiled.(*cs.R1CS)
	if !ok {
		t.Fatal("compiled circuit is not BN254 R1CS")
	}
	domain := ecc.NextPowerOfTwo(uint64(typed.GetNbConstraints()))
	phase1 := new(mpc.Phase1)
	phase1.Initialize(domain)
	phase1.Contribute()
	first1 := new(mpc.Phase1)
	snapshot(phase1, first1)
	phase1.Contribute()
	second1 := new(mpc.Phase1)
	snapshot(phase1, second1)
	commons, err := mpc.VerifyPhase1(domain, []byte("post-phase1-beacon"), first1, second1)
	if err != nil {
		t.Fatal(err)
	}
	phase2 := new(mpc.Phase2)
	phase2.Initialize(typed, &commons)
	phase2.Contribute()
	first2 := new(mpc.Phase2)
	snapshot(phase2, first2)
	phase2.Contribute()
	second2 := new(mpc.Phase2)
	snapshot(phase2, second2)
	pk, vk, err := mpc.VerifyPhase2(
		typed, &commons, []byte("post-phase2-beacon"), first2, second2,
	)
	if err != nil {
		t.Fatal(err)
	}
	dump := new(bytes.Buffer)
	if err := pk.WriteDump(dump); err != nil {
		t.Fatal(err)
	}
	reloaded := groth16.NewProvingKey(ecc.BN254)
	if err := reloaded.ReadDump(bytes.NewReader(dump.Bytes())); err != nil {
		t.Fatal(err)
	}
	witness, err := frontend.NewWitness(
		&squareCircuit{Square: 9, Root: 3}, ecc.BN254.ScalarField(),
	)
	if err != nil {
		t.Fatal(err)
	}
	proof, err := groth16.Prove(compiled, reloaded, witness)
	if err != nil {
		t.Fatal(err)
	}
	publicWitness, err := witness.Public()
	if err != nil {
		t.Fatal(err)
	}
	if err := groth16.Verify(proof, vk, publicWitness); err != nil {
		t.Fatal(err)
	}
}
