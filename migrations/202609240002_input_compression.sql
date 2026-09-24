-- Large random sample arrays spent substantially less time in COPY with LZ4.
-- This affects future input writes; existing payloads keep their encoding.
DO $$
BEGIN
    ALTER TABLE batch_inputs ALTER COLUMN latent_batch SET COMPRESSION lz4;
EXCEPTION WHEN feature_not_supported THEN
    RAISE WARNING 'PostgreSQL lacks LZ4 support; batch input compression is unchanged';
END $$;
