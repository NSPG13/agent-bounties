#!/usr/bin/env bash
exec bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_shared/run-python.sh" github_issue_plan_comment.py "$@"
