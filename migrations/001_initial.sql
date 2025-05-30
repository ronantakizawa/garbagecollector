-- Create leases table
CREATE TABLE IF NOT EXISTS leases (
    lease_id VARCHAR(36) PRIMARY KEY,
    object_id VARCHAR(255) NOT NULL,
    object_type INTEGER NOT NULL,
    service_id VARCHAR(255) NOT NULL,
    state INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    last_renewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}',
    cleanup_config JSONB,
    renewal_count INTEGER NOT NULL DEFAULT 0
);

-- Create cleanup_history table for tracking cleanup operations
CREATE TABLE IF NOT EXISTS cleanup_history (
    id SERIAL PRIMARY KEY,
    lease_id VARCHAR(36) NOT NULL,
    object_id VARCHAR(255) NOT NULL,
    service_id VARCHAR(255) NOT NULL,
    cleanup_started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    cleanup_completed_at TIMESTAMPTZ,
    success BOOLEAN NOT NULL DEFAULT FALSE,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0
);

-- Create service_stats table for tracking per-service metrics
CREATE TABLE IF NOT EXISTS service_stats (
    service_id VARCHAR(255) PRIMARY KEY,
    total_leases_created INTEGER NOT NULL DEFAULT 0,
    total_leases_renewed INTEGER NOT NULL DEFAULT 0,
    total_leases_expired INTEGER NOT NULL DEFAULT 0,
    total_leases_released INTEGER NOT NULL DEFAULT 0,
    current_active_leases INTEGER NOT NULL DEFAULT 0,
    last_activity_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Add indexes for common queries (outside the CREATE TABLE statements)
CREATE INDEX IF NOT EXISTS idx_leases_service_id ON leases (service_id);
CREATE INDEX IF NOT EXISTS idx_leases_object_type ON leases (object_type);
CREATE INDEX IF NOT EXISTS idx_leases_state ON leases (state);
CREATE INDEX IF NOT EXISTS idx_leases_expires_at ON leases (expires_at);
CREATE INDEX IF NOT EXISTS idx_leases_created_at ON leases (created_at);
CREATE INDEX IF NOT EXISTS idx_leases_service_state ON leases (service_id, state);
CREATE INDEX IF NOT EXISTS idx_leases_state_expires_at ON leases (state, expires_at);

CREATE INDEX IF NOT EXISTS idx_cleanup_history_lease_id ON cleanup_history (lease_id);
CREATE INDEX IF NOT EXISTS idx_cleanup_history_service_id ON cleanup_history (service_id);
CREATE INDEX IF NOT EXISTS idx_cleanup_history_started_at ON cleanup_history (cleanup_started_at);

CREATE INDEX IF NOT EXISTS idx_service_stats_last_activity ON service_stats (last_activity_at);
CREATE INDEX IF NOT EXISTS idx_service_stats_active_leases ON service_stats (current_active_leases);

-- These partial indexes do not use NOW() and are legal.
CREATE INDEX IF NOT EXISTS idx_leases_service_active 
    ON leases (service_id) WHERE state = 0;

CREATE INDEX IF NOT EXISTS idx_leases_cleanup_ready 
    ON leases (expires_at, state) WHERE state = 0;

-- No index with NOW() or any non-IMMUTABLE function in WHERE clause!

-- Create function to update service stats automatically
CREATE OR REPLACE FUNCTION update_service_stats() RETURNS TRIGGER AS $$
BEGIN
    -- Handle INSERT (new lease created)
    IF TG_OP = 'INSERT' THEN
        INSERT INTO service_stats (service_id, total_leases_created, current_active_leases)
        VALUES (NEW.service_id, 1, 1)
        ON CONFLICT (service_id) DO UPDATE SET
            total_leases_created = service_stats.total_leases_created + 1,
            current_active_leases = service_stats.current_active_leases + 1,
            last_activity_at = NOW();
        RETURN NEW;
    END IF;
    
    -- Handle UPDATE (lease state change)
    IF TG_OP = 'UPDATE' THEN
        -- If lease was renewed
        IF NEW.last_renewed_at > OLD.last_renewed_at THEN
            UPDATE service_stats 
            SET total_leases_renewed = total_leases_renewed + 1,
                last_activity_at = NOW()
            WHERE service_id = NEW.service_id;
        END IF;
        
        -- If lease state changed from active to expired
        IF OLD.state = 0 AND NEW.state = 1 THEN
            UPDATE service_stats 
            SET total_leases_expired = total_leases_expired + 1,
                current_active_leases = current_active_leases - 1,
                last_activity_at = NOW()
            WHERE service_id = NEW.service_id;
        END IF;
        
        -- If lease state changed from active to released
        IF OLD.state = 0 AND NEW.state = 2 THEN
            UPDATE service_stats 
            SET total_leases_released = total_leases_released + 1,
                current_active_leases = current_active_leases - 1,
                last_activity_at = NOW()
            WHERE service_id = NEW.service_id;
        END IF;
        
        RETURN NEW;
    END IF;
    
    -- Handle DELETE
    IF TG_OP = 'DELETE' THEN
        -- If deleting an active lease, decrease the count
        IF OLD.state = 0 THEN
            UPDATE service_stats 
            SET current_active_leases = current_active_leases - 1,
                last_activity_at = NOW()
            WHERE service_id = OLD.service_id;
        END IF;
        RETURN OLD;
    END IF;
    
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- Create trigger to automatically update service stats
DROP TRIGGER IF EXISTS trigger_update_service_stats ON leases;
CREATE TRIGGER trigger_update_service_stats
    AFTER INSERT OR UPDATE OR DELETE ON leases
    FOR EACH ROW EXECUTE FUNCTION update_service_stats();

-- Create function to clean up old cleanup history
CREATE OR REPLACE FUNCTION cleanup_old_history() RETURNS INTEGER AS $$
DECLARE
    deleted_count INTEGER;
BEGIN
    -- Delete cleanup history older than 30 days
    DELETE FROM cleanup_history 
    WHERE cleanup_started_at < NOW() - INTERVAL '30 days';
    
    GET DIAGNOSTICS deleted_count = ROW_COUNT;
    RETURN deleted_count;
END;
$$ LANGUAGE plpgsql;

-- Create function to get lease statistics
CREATE OR REPLACE FUNCTION get_lease_statistics() 
RETURNS TABLE(
    total_leases BIGINT,
    active_leases BIGINT,
    expired_leases BIGINT,
    released_leases BIGINT,
    avg_lease_duration_hours NUMERIC,
    avg_renewal_count NUMERIC
) AS $$
BEGIN
    RETURN QUERY
    SELECT 
        COUNT(*) as total_leases,
        COUNT(*) FILTER (WHERE state = 0 AND expires_at > NOW()) as active_leases,
        COUNT(*) FILTER (WHERE state = 1 OR (state = 0 AND expires_at <= NOW())) as expired_leases,
        COUNT(*) FILTER (WHERE state = 2) as released_leases,
        AVG(EXTRACT(EPOCH FROM (expires_at - created_at)) / 3600) as avg_lease_duration_hours,
        AVG(renewal_count) as avg_renewal_count
    FROM leases;
END;
$$ LANGUAGE plpgsql;

-- Comments for documentation
COMMENT ON TABLE leases IS 'Stores lease information for distributed garbage collection';
COMMENT ON TABLE cleanup_history IS 'Tracks cleanup operations and their results';
COMMENT ON TABLE service_stats IS 'Aggregated statistics per service';
COMMENT ON FUNCTION update_service_stats() IS 'Automatically updates service statistics when leases change';
COMMENT ON FUNCTION cleanup_old_history() IS 'Removes old cleanup history records';
COMMENT ON FUNCTION get_lease_statistics() IS 'Returns comprehensive lease statistics';
