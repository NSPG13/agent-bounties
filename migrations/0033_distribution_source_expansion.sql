ALTER TABLE distribution_acquisitions
  DROP CONSTRAINT IF EXISTS distribution_acquisition_rail_check,
  ADD CONSTRAINT distribution_acquisition_rail_check CHECK (
    first_touch_rail IN (
      'bankr', 'github', 'linear', 'vscode', 'cursor', 'cline',
      'openclaw', 'claude-custom', 'chatgpt-dev', 'glama',
      'mcp-so', 'mcpservers', 'glama-paid', 'mcp-so-paid'
    )
  );

ALTER TABLE distribution_acquisition_assists
  DROP CONSTRAINT IF EXISTS distribution_assist_rail_check,
  ADD CONSTRAINT distribution_assist_rail_check CHECK (
    rail IN (
      'bankr', 'github', 'linear', 'vscode', 'cursor', 'cline',
      'openclaw', 'claude-custom', 'chatgpt-dev', 'glama',
      'mcp-so', 'mcpservers', 'glama-paid', 'mcp-so-paid'
    )
  );

ALTER TABLE distribution_rail_usage_hourly
  DROP CONSTRAINT IF EXISTS distribution_rail_usage_rail_check,
  ADD CONSTRAINT distribution_rail_usage_rail_check CHECK (
    rail IN (
      'bankr', 'github', 'linear', 'vscode', 'cursor', 'cline',
      'openclaw', 'claude-custom', 'chatgpt-dev', 'glama',
      'mcp-so', 'mcpservers', 'glama-paid', 'mcp-so-paid'
    )
  );
