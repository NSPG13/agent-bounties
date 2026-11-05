CREATE TABLE IF NOT EXISTS gas_sponsorship_daily_budget (
    budget_key text NOT NULL,
    budget_day date NOT NULL,
    reserved_micro_usdc bigint NOT NULL CHECK (reserved_micro_usdc >= 0),
    limit_micro_usdc bigint NOT NULL CHECK (limit_micro_usdc > 0),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (budget_key, budget_day),
    CHECK (reserved_micro_usdc <= limit_micro_usdc)
);
