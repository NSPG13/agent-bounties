package main

import (
	"bufio"
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/groth16"
	mpc "github.com/consensys/gnark/backend/groth16/bn254/mpcsetup"
	cs "github.com/consensys/gnark/constraint/bn254"
)

const schema = "agent-bounties/open-competition-v2-beta2-groth16-mpc-command-v1"
const ioBufferSize = 16 << 20

type record struct {
	Schema         string   `json:"schema_version"`
	Command        string   `json:"command"`
	Inputs         []digest `json:"inputs"`
	Outputs        []digest `json:"outputs"`
	ContributionID int      `json:"contribution_id,omitempty"`
	DomainSize     uint64   `json:"domain_size,omitempty"`
	BeaconHex      string   `json:"beacon_hex,omitempty"`
	Verified       bool     `json:"verified"`
}

type digest struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
	Bytes  int64  `json:"bytes"`
}

func main() {
	if len(os.Args) < 2 {
		fail(errors.New("usage: ceremony <init-phase1|contribute-phase1|verify-phase1|init-phase2|contribute-phase2|finalize> ..."))
	}
	command := os.Args[1]
	var result record
	var err error
	switch command {
	case "init-phase1":
		result, err = initPhase1(os.Args[2:])
	case "contribute-phase1":
		result, err = contributePhase1(os.Args[2:])
	case "verify-phase1":
		result, err = verifyPhase1(os.Args[2:])
	case "init-phase2":
		result, err = initPhase2(os.Args[2:])
	case "contribute-phase2":
		result, err = contributePhase2(os.Args[2:])
	case "finalize":
		result, err = finalize(os.Args[2:])
	default:
		err = fmt.Errorf("unknown command %q", command)
	}
	if err != nil {
		fail(err)
	}
	encoded, err := json.Marshal(result)
	if err != nil {
		fail(err)
	}
	fmt.Println(string(encoded))
}

func initPhase1(args []string) (record, error) {
	flags := flag.NewFlagSet("init-phase1", flag.ContinueOnError)
	r1csPath := flags.String("r1cs", "", "exact Groth16 circuit")
	output := flags.String("output", "", "initial Phase 1 transcript")
	if err := flags.Parse(args); err != nil {
		return record{}, err
	}
	r1cs, err := readR1CS(*r1csPath)
	if err != nil {
		return record{}, err
	}
	domainSize := ecc.NextPowerOfTwo(uint64(r1cs.GetNbConstraints()))
	phase := new(mpc.Phase1)
	phase.Initialize(domainSize)
	if err := writeTo(*output, phase); err != nil {
		return record{}, err
	}
	return makeRecord("init-phase1", []string{*r1csPath}, []string{*output}, 0, domainSize, "")
}

func contributePhase1(args []string) (record, error) {
	flags := flag.NewFlagSet("contribute-phase1", flag.ContinueOnError)
	input := flags.String("input", "", "previous Phase 1 transcript")
	output := flags.String("output", "", "contributed Phase 1 transcript")
	id := flags.Int("contribution-id", 0, "strictly positive contribution sequence")
	if err := flags.Parse(args); err != nil {
		return record{}, err
	}
	if *id < 1 {
		return record{}, errors.New("contribution-id must be positive")
	}
	phase := new(mpc.Phase1)
	if err := readFrom(*input, phase); err != nil {
		return record{}, err
	}
	phase.Contribute()
	if err := writeToExclusive(*output, phase); err != nil {
		return record{}, err
	}
	return makeRecord("contribute-phase1", []string{*input}, []string{*output}, *id, 0, "")
}

func verifyPhase1(args []string) (record, error) {
	flags := flag.NewFlagSet("verify-phase1", flag.ContinueOnError)
	r1csPath := flags.String("r1cs", "", "exact Groth16 circuit")
	inputs := flags.String("inputs", "", "ordered comma-separated Phase 1 contributions")
	beacon := flags.String("beacon", "", "0x-prefixed post-contribution public beacon")
	output := flags.String("output", "", "verified common reference string")
	if err := flags.Parse(args); err != nil {
		return record{}, err
	}
	r1cs, err := readR1CS(*r1csPath)
	if err != nil {
		return record{}, err
	}
	paths, phases, err := readPhase1List(*inputs)
	if err != nil {
		return record{}, err
	}
	beaconBytes, canonicalBeacon, err := parseBeacon(*beacon)
	if err != nil {
		return record{}, err
	}
	domainSize := ecc.NextPowerOfTwo(uint64(r1cs.GetNbConstraints()))
	commons, err := mpc.VerifyPhase1(domainSize, beaconBytes, phases...)
	if err != nil {
		return record{}, fmt.Errorf("Phase 1 verification failed: %w", err)
	}
	if err := writeTo(*output, &commons); err != nil {
		return record{}, err
	}
	return makeRecord("verify-phase1", append([]string{*r1csPath}, paths...), []string{*output}, 0, domainSize, canonicalBeacon)
}

func initPhase2(args []string) (record, error) {
	flags := flag.NewFlagSet("init-phase2", flag.ContinueOnError)
	r1csPath := flags.String("r1cs", "", "exact Groth16 circuit")
	commonsPath := flags.String("commons", "", "verified Phase 1 commons")
	output := flags.String("output", "", "initial Phase 2 transcript")
	if err := flags.Parse(args); err != nil {
		return record{}, err
	}
	r1cs, err := readR1CS(*r1csPath)
	if err != nil {
		return record{}, err
	}
	commons := new(mpc.SrsCommons)
	if err := readFrom(*commonsPath, commons); err != nil {
		return record{}, err
	}
	phase := new(mpc.Phase2)
	phase.Initialize(r1cs, commons)
	if err := writeTo(*output, phase); err != nil {
		return record{}, err
	}
	return makeRecord("init-phase2", []string{*r1csPath, *commonsPath}, []string{*output}, 0, 0, "")
}

func contributePhase2(args []string) (record, error) {
	flags := flag.NewFlagSet("contribute-phase2", flag.ContinueOnError)
	input := flags.String("input", "", "previous Phase 2 transcript")
	output := flags.String("output", "", "contributed Phase 2 transcript")
	id := flags.Int("contribution-id", 0, "strictly positive contribution sequence")
	if err := flags.Parse(args); err != nil {
		return record{}, err
	}
	if *id < 1 {
		return record{}, errors.New("contribution-id must be positive")
	}
	phase := new(mpc.Phase2)
	if err := readFrom(*input, phase); err != nil {
		return record{}, err
	}
	phase.Contribute()
	if err := writeToExclusive(*output, phase); err != nil {
		return record{}, err
	}
	return makeRecord("contribute-phase2", []string{*input}, []string{*output}, *id, 0, "")
}

func finalize(args []string) (record, error) {
	flags := flag.NewFlagSet("finalize", flag.ContinueOnError)
	r1csPath := flags.String("r1cs", "", "exact Groth16 circuit")
	commonsPath := flags.String("commons", "", "verified Phase 1 commons")
	inputs := flags.String("inputs", "", "ordered comma-separated Phase 2 contributions")
	beacon := flags.String("beacon", "", "0x-prefixed post-contribution public beacon")
	pkPath := flags.String("pk", "", "output proving key")
	vkPath := flags.String("vk", "", "output verifying key")
	solidityPath := flags.String("solidity", "", "output Solidity verifier")
	if err := flags.Parse(args); err != nil {
		return record{}, err
	}
	r1cs, err := readR1CS(*r1csPath)
	if err != nil {
		return record{}, err
	}
	commons := new(mpc.SrsCommons)
	if err := readFrom(*commonsPath, commons); err != nil {
		return record{}, err
	}
	paths, phases, err := readPhase2List(*inputs)
	if err != nil {
		return record{}, err
	}
	beaconBytes, canonicalBeacon, err := parseBeacon(*beacon)
	if err != nil {
		return record{}, err
	}
	pk, vk, err := mpc.VerifyPhase2(r1cs, commons, beaconBytes, phases...)
	if err != nil {
		return record{}, fmt.Errorf("Phase 2 verification failed: %w", err)
	}
	if err := writeDumpExclusive(*pkPath, pk); err != nil {
		return record{}, err
	}
	if err := writeToExclusive(*vkPath, vk); err != nil {
		return record{}, err
	}
	if err := writeSolidity(*solidityPath, vk); err != nil {
		return record{}, err
	}
	return makeRecord(
		"finalize",
		append([]string{*r1csPath, *commonsPath}, paths...),
		[]string{*pkPath, *vkPath, *solidityPath},
		0,
		0,
		canonicalBeacon,
	)
}

func readR1CS(path string) (*cs.R1CS, error) {
	if path == "" {
		return nil, errors.New("r1cs path is required")
	}
	value := groth16.NewCS(ecc.BN254)
	if err := readFrom(path, value); err != nil {
		return nil, err
	}
	typed, ok := value.(*cs.R1CS)
	if !ok {
		return nil, errors.New("Groth16 circuit is not BN254 R1CS")
	}
	return typed, nil
}

func readPhase1List(raw string) ([]string, []*mpc.Phase1, error) {
	paths, err := splitPaths(raw)
	if err != nil {
		return nil, nil, err
	}
	values := make([]*mpc.Phase1, len(paths))
	for i, path := range paths {
		values[i] = new(mpc.Phase1)
		if err := readFrom(path, values[i]); err != nil {
			return nil, nil, err
		}
	}
	return paths, values, nil
}

func readPhase2List(raw string) ([]string, []*mpc.Phase2, error) {
	paths, err := splitPaths(raw)
	if err != nil {
		return nil, nil, err
	}
	values := make([]*mpc.Phase2, len(paths))
	for i, path := range paths {
		values[i] = new(mpc.Phase2)
		if err := readFrom(path, values[i]); err != nil {
			return nil, nil, err
		}
	}
	return paths, values, nil
}

func splitPaths(raw string) ([]string, error) {
	if raw == "" {
		return nil, errors.New("at least one contribution is required")
	}
	paths := strings.Split(raw, ",")
	for _, path := range paths {
		if strings.TrimSpace(path) == "" || path != strings.TrimSpace(path) {
			return nil, errors.New("contribution paths must be nonempty and whitespace-free")
		}
	}
	return paths, nil
}

func parseBeacon(raw string) ([]byte, string, error) {
	if !strings.HasPrefix(raw, "0x") || len(raw) < 66 || len(raw)%2 != 0 {
		return nil, "", errors.New("beacon must be lowercase 0x-prefixed bytes with at least 32 bytes")
	}
	if raw != strings.ToLower(raw) {
		return nil, "", errors.New("beacon must use lowercase hex")
	}
	value, err := hex.DecodeString(raw[2:])
	if err != nil {
		return nil, "", errors.New("beacon must contain only hex digits")
	}
	return value, raw, nil
}

func readFrom(path string, value io.ReaderFrom) error {
	if path == "" {
		return errors.New("input path is required")
	}
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer file.Close()
	_, err = value.ReadFrom(bufio.NewReaderSize(file, ioBufferSize))
	return err
}

func writeTo(path string, value io.WriterTo) error {
	if path == "" {
		return errors.New("output path is required")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o600)
	if err != nil {
		return err
	}
	defer file.Close()
	writer := bufio.NewWriterSize(file, ioBufferSize)
	if _, err = value.WriteTo(writer); err != nil {
		return err
	}
	return writer.Flush()
}

func writeToExclusive(path string, value io.WriterTo) error {
	if path == "" {
		return errors.New("output path is required")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	defer file.Close()
	writer := bufio.NewWriterSize(file, ioBufferSize)
	if _, err = value.WriteTo(writer); err != nil {
		return err
	}
	return writer.Flush()
}

type dumpWriter interface {
	WriteDump(io.Writer) error
}

func writeDumpExclusive(path string, value dumpWriter) error {
	if path == "" {
		return errors.New("output path is required")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	defer file.Close()
	writer := bufio.NewWriterSize(file, ioBufferSize)
	if err := value.WriteDump(writer); err != nil {
		return err
	}
	return writer.Flush()
}

func writeSolidity(path string, vk groth16.VerifyingKey) error {
	if path == "" {
		return errors.New("Solidity output path is required")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	buffer := new(bytes.Buffer)
	if err := vk.ExportSolidity(buffer); err != nil {
		return err
	}
	content, err := normalizeGroth16Solidity(buffer.String())
	if err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
	if err != nil {
		return err
	}
	defer file.Close()
	_, err = io.WriteString(file, content)
	return err
}

func normalizeGroth16Solidity(content string) (string, error) {
	replacements := [][2]string{
		{"pragma solidity ^0.8.0;", "pragma solidity ^0.8.20;"},
		{"contract Verifier {", "contract Groth16Verifier {"},
		{"function verifyProof(", "function Verify("},
	}
	for _, replacement := range replacements {
		if strings.Count(content, replacement[0]) != 1 {
			return "", fmt.Errorf("expected exactly one Solidity fragment %q", replacement[0])
		}
		content = strings.Replace(content, replacement[0], replacement[1], 1)
	}
	return content, nil
}

func makeRecord(command string, inputs, outputs []string, id int, domain uint64, beacon string) (record, error) {
	inputDigests, err := digests(inputs)
	if err != nil {
		return record{}, err
	}
	outputDigests, err := digests(outputs)
	if err != nil {
		return record{}, err
	}
	return record{
		Schema: schema, Command: command, Inputs: inputDigests, Outputs: outputDigests,
		ContributionID: id, DomainSize: domain, BeaconHex: beacon, Verified: true,
	}, nil
}

func digests(paths []string) ([]digest, error) {
	result := make([]digest, len(paths))
	for i, path := range paths {
		file, err := os.Open(path)
		if err != nil {
			return nil, err
		}
		hash := sha256.New()
		bytes, copyErr := io.Copy(hash, file)
		closeErr := file.Close()
		if copyErr != nil {
			return nil, copyErr
		}
		if closeErr != nil {
			return nil, closeErr
		}
		result[i] = digest{Path: filepath.Base(path), SHA256: hex.EncodeToString(hash.Sum(nil)), Bytes: bytes}
	}
	return result, nil
}

func fail(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
