-- Initialisation des tables enedis-rs pour PostgreSQL
CREATE TABLE IF NOT EXISTS measurements (
    point_id VARCHAR(14) NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL,
    direction VARCHAR(16) NOT NULL,
    interval_seconds INTEGER NOT NULL,
    value NUMERIC(18, 4) NOT NULL,
    unit VARCHAR(16) NOT NULL,
    quality SMALLINT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (point_id, timestamp, direction)
);

CREATE INDEX IF NOT EXISTS idx_measurements_point_range 
ON measurements (point_id, timestamp DESC);

-- Support optionnel TimescaleDB (si l'extension est présente sur le serveur Postgres) :
-- CREATE EXTENSION IF NOT EXISTS timescaledb CASCADE;
-- SELECT create_hypertable('measurements', 'timestamp', if_not_exists => TRUE, chunk_time_interval => INTERVAL '1 month');

CREATE TABLE IF NOT EXISTS sync_state (
    point_id VARCHAR(14) NOT NULL,
    direction VARCHAR(16) NOT NULL,
    last_synced_timestamp TIMESTAMPTZ NOT NULL,
    last_sync_attempt TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    sync_status VARCHAR(32) NOT NULL DEFAULT 'OK',
    PRIMARY KEY (point_id, direction)
);

-- Données de démonstration : 2 PRMs (un résidentiel classique en soutirage, un prosumer avec PV)
INSERT INTO sync_state (point_id, direction, last_synced_timestamp, last_sync_attempt, sync_status)
VALUES 
    ('01234567890123', 'Consumption', CURRENT_TIMESTAMP - INTERVAL '1 hour', CURRENT_TIMESTAMP, 'OK'),
    ('09876543210987', 'Consumption', CURRENT_TIMESTAMP - INTERVAL '2 hours', CURRENT_TIMESTAMP, 'OK'),
    ('09876543210987', 'Production', CURRENT_TIMESTAMP - INTERVAL '2 hours', CURRENT_TIMESTAMP, 'OK')
ON CONFLICT (point_id, direction) DO NOTHING;

-- Génération de 7 jours de mesures au pas 30 minutes (336 points par PRM)
-- Profil Résidentiel (01234567890123) : profil avec consommation en journée et soirée
INSERT INTO measurements (point_id, timestamp, direction, interval_seconds, value, unit, quality)
SELECT 
    '01234567890123' AS point_id,
    series_ts AS timestamp,
    'Consumption' AS direction,
    1800 AS interval_seconds,
    -- Profil réaliste bureau : base de nuit 2.5 kW + pic de travail 12-18 kW entre 8h et 19h
    ROUND(
        (2.5 + 
        CASE 
            WHEN EXTRACT(DOW FROM series_ts) IN (0, 6) THEN 0.5 -- Week-end bas
            WHEN EXTRACT(HOUR FROM series_ts) BETWEEN 8 AND 18 THEN 10.0 + 3.0 * SIN(EXTRACT(HOUR FROM series_ts) * 3.14 / 12)
            ELSE 1.0
        END + (RANDOM() * 0.8))::numeric, 4
    ) AS value,
    'kW' AS unit,
    -- Qualité : 3 (Mesuré) pour les jours passés, 2 ou 1 pour le plus récent
    CASE 
        WHEN series_ts < CURRENT_TIMESTAMP - INTERVAL '2 days' THEN 3
        WHEN series_ts < CURRENT_TIMESTAMP - INTERVAL '1 day' THEN 2
        ELSE 1
    END AS quality
FROM generate_series(
    DATE_TRUNC('hour', CURRENT_TIMESTAMP - INTERVAL '7 days'),
    DATE_TRUNC('hour', CURRENT_TIMESTAMP),
    INTERVAL '30 minutes'
) AS series_ts
ON CONFLICT (point_id, timestamp, direction) DO NOTHING;

-- Profil Prosumer PV (09876543210987) : soutirage résidentiel + injection solaire
INSERT INTO measurements (point_id, timestamp, direction, interval_seconds, value, unit, quality)
SELECT 
    '09876543210987' AS point_id,
    series_ts AS timestamp,
    'Consumption' AS direction,
    1800 AS interval_seconds,
    ROUND(
        (0.6 + 
        CASE 
            WHEN EXTRACT(HOUR FROM series_ts) IN (6, 7, 8, 19, 20, 21, 22) THEN 2.2 + RANDOM() * 1.5 -- Matin et soir
            ELSE 0.4 + RANDOM() * 0.3
        END)::numeric, 4
    ) AS value,
    'kW' AS unit,
    3 AS quality
FROM generate_series(
    DATE_TRUNC('hour', CURRENT_TIMESTAMP - INTERVAL '7 days'),
    DATE_TRUNC('hour', CURRENT_TIMESTAMP),
    INTERVAL '30 minutes'
) AS series_ts
ON CONFLICT (point_id, timestamp, direction) DO NOTHING;

-- Injection Solaire (Production)
INSERT INTO measurements (point_id, timestamp, direction, interval_seconds, value, unit, quality)
SELECT 
    '09876543210987' AS point_id,
    series_ts AS timestamp,
    'Production' AS direction,
    1800 AS interval_seconds,
    ROUND(
        (CASE 
            WHEN EXTRACT(HOUR FROM series_ts) BETWEEN 9 AND 17 THEN 
                GREATEST(0.0, 4.5 * SIN((EXTRACT(HOUR FROM series_ts) - 8) * 3.14 / 10) + (RANDOM() * 0.5 - 0.25))
            ELSE 0.0
        END)::numeric, 4
    ) AS value,
    'kW' AS unit,
    3 AS quality
FROM generate_series(
    DATE_TRUNC('hour', CURRENT_TIMESTAMP - INTERVAL '7 days'),
    DATE_TRUNC('hour', CURRENT_TIMESTAMP),
    INTERVAL '30 minutes'
) AS series_ts
WHERE EXTRACT(HOUR FROM series_ts) BETWEEN 9 AND 17
ON CONFLICT (point_id, timestamp, direction) DO NOTHING;
