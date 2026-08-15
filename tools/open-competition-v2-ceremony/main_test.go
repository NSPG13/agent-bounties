package main

import (
	"bytes"
	"io"
	"path/filepath"
	"testing"
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
