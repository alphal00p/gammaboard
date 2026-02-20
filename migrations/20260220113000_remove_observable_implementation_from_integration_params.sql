-- Observable selection is now derived from evaluator_implementation.
-- Remove redundant per-run observable_implementation key from integration_params.

UPDATE runs
SET integration_params = integration_params - 'observable_implementation'
WHERE integration_params ? 'observable_implementation';
