CREATE TABLE IF NOT EXISTS invoice_orders (
  order_id TEXT PRIMARY KEY,
  buyer_name TEXT NOT NULL,
  buyer_email TEXT NOT NULL,
  stripe_customer_id TEXT,
  stripe_invoice_id TEXT UNIQUE,
  hosted_invoice_url TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS invoice_order_events (
  order_id TEXT NOT NULL REFERENCES invoice_orders (order_id),
  sequence INTEGER NOT NULL CHECK (sequence >= 0),
  kind TEXT NOT NULL,
  event JSONB NOT NULL,
  evidence_id TEXT UNIQUE,
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (order_id, sequence)
);

CREATE TABLE IF NOT EXISTS invoice_contractors (
  contractor_id TEXT PRIMARY KEY,
  wallet TEXT NOT NULL UNIQUE,
  record JSONB NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
