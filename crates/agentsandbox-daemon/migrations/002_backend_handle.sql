-- Add opaque backend runtime handle required by the plugin registry.
-- This is intentionally nullable so existing rows remain valid.

ALTER TABLE sandboxes ADD COLUMN backend_handle TEXT;
