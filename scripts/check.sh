#!/usr/bin/env bash
exec bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_shared/run-python.sh" check.py --platform posix "$@"
