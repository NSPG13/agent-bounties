"""Error types for the Taskmarket adapter.

`TaskmarketError` is the base class for every refusal or failure this package
raises. Messages are written so they can be returned to an untrusted MCP
caller without leaking host paths, CLI output, or provider details.
"""
from __future__ import annotations


class TaskmarketError(RuntimeError):
    """Base error; message is safe to show to an untrusted caller."""


class AuthorizationError(TaskmarketError):
    """A required operator authorization artifact is missing, invalid, expired,
    scoped to another action, or does not cover the requested spend."""


class SecurityError(TaskmarketError):
    """A requested artifact path or network violates host policy."""
