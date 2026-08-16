FROM golang:1.26@sha256:26326682769ca980f8f1d3b1f52be2dd1c1d25270e3de3fe0c97d6bb65df3556 AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY phase1-parallel.patch phase1_parallel_test.go.txt /tmp/
RUN module_dir="$(go env GOMODCACHE)/github.com/p4u/gnark@v0.0.0-20251217225531-cd7874155e26" \
    && chmod -R u+w "$module_dir" \
    && cd "$module_dir" \
    && git apply --unidiff-zero /tmp/phase1-parallel.patch \
    && cp /tmp/phase1_parallel_test.go.txt backend/groth16/bn254/mpcsetup/phase1_parallel_test.go \
    && go test ./backend/groth16/bn254/mpcsetup -run 'TestParallel(UpdateMatchesSequential|CompressedRead)' -count=1
COPY main.go main_test.go ./
RUN go test ./... -count=1 \
    && CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -ldflags="-s -w" -o /ceremony .

FROM scratch
COPY --from=build /ceremony /ceremony
ENTRYPOINT ["/ceremony"]
