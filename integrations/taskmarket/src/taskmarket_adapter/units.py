"""Exact USDC Decimal <-> base-unit conversion.

The official Taskmarket CLI takes human-readable USDC strings with at most six
decimal places on its `--reward` / price flags, while the chain and APIs use
integer base units (1 USDC = 1_000_000 base units). Mixing the two up is a
six-order-of-magnitude spend bug, so all conversion lives here and is tested.
"""
from __future__ import annotations

from decimal import Decimal, InvalidOperation
from typing import Union

from .errors import TaskmarketError

BASE_UNITS_PER_USDC = 1_000_000
_SCALE = Decimal(BASE_UNITS_PER_USDC)
_MAX_DECIMALS = 6

UsdcInput = Union[str, int, Decimal]


def parse_usdc(value: UsdcInput) -> Decimal:
    """Parse a human-readable USDC amount into an exact Decimal.

    Accepts "5", "5.5", "0.000001", integers, or Decimals. Rejects negative,
    zero, NaN, infinite, and values with more than six decimal places.
    """
    if isinstance(value, bool) or not isinstance(value, (str, int, Decimal)):
        raise TaskmarketError("invalid reward: expected a human-readable USDC amount")
    text = str(value).strip()
    if "e" in text.lower():
        # Exponent notation is ambiguous at the spend boundary; require plain decimals.
        raise TaskmarketError("invalid reward: expected a plain decimal USDC amount")
    try:
        amount = Decimal(text)
    except (InvalidOperation, ValueError):
        raise TaskmarketError("invalid reward: expected a human-readable USDC amount") from None
    if not amount.is_finite():
        raise TaskmarketError("invalid reward: must be finite")
    if -amount.as_tuple().exponent > _MAX_DECIMALS:  # type: ignore[operator]
        raise TaskmarketError("invalid reward: at most six decimal places are supported")
    if amount <= 0:
        raise TaskmarketError("invalid reward: must be greater than zero")
    return amount


def usdc_to_base_units(value: UsdcInput) -> int:
    """Convert human-readable USDC to integer base units exactly."""
    return int(parse_usdc(value) * _SCALE)


def base_units_to_usdc(units: int) -> Decimal:
    """Convert integer base units to a human-readable USDC Decimal."""
    if isinstance(units, bool) or not isinstance(units, int):
        raise TaskmarketError("invalid base units: expected an integer")
    if units <= 0:
        raise TaskmarketError("invalid base units: must be greater than zero")
    return Decimal(units) / _SCALE


def format_usdc(amount: Decimal) -> str:
    """Render a USDC Decimal as the canonical CLI flag value (no trailing zeros)."""
    text = format(amount.normalize(), "f")
    if "." in text:
        whole, frac = text.split(".", 1)
        frac = frac.rstrip("0")
        text = whole if not frac else f"{whole}.{frac}"
    return text


def usdc_flag_value(value: UsdcInput) -> str:
    """Validated, canonical string for a CLI `--reward` style flag."""
    return format_usdc(parse_usdc(value))
