ALTER TABLE chatgpt_action_intents
  DROP CONSTRAINT IF EXISTS chatgpt_action_intents_action_check;

UPDATE chatgpt_action_intents
SET action = 'solve',
    updated_at = now()
WHERE action = 'compete';

ALTER TABLE chatgpt_action_intents
  ADD CONSTRAINT chatgpt_action_intents_action_check CHECK (
    action IN ('post', 'fund', 'solve', 'complete', 'verify')
  );
