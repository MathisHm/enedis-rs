#!/usr/bin/env bash
set -e

echo "=========================================================="
echo "⚡ Démarrage de l'Add-on Enedis Linky Gateway pour Home Assistant"
echo "=========================================================="

OPTIONS_PATH="/data/options.json"

if [ ! -f "$OPTIONS_PATH" ]; then
    echo "⚠️ Fichier de configuration /data/options.json non trouvé, utilisation des variables d'environnement."
fi

# Extraction des options avec jq (avec valeurs de repli)
get_opt() {
    local key="$1"
    local default="$2"
    if [ -f "$OPTIONS_PATH" ]; then
        local val
        val=$(jq -r ".${key} // empty" "$OPTIONS_PATH")
        if [ -n "$val" ] && [ "$val" != "null" ]; then
            echo "$val"
            return
        fi
    fi
    echo "$default"
}

PRM=$(get_opt "prm" "")
PROVIDER=$(get_opt "provider" "dataconnect")
DC_CLIENT_ID=$(get_opt "dataconnect_client_id" "")
DC_CLIENT_SECRET=$(get_opt "dataconnect_client_secret" "")
SGE_CERT=$(get_opt "sge_cert_pem_path" "")
SGE_KEY=$(get_opt "sge_key_pem_path" "")

MQTT_HOST=$(get_opt "mqtt_host" "core-mosquitto")
MQTT_PORT=$(get_opt "mqtt_port" "1883")
MQTT_USER=$(get_opt "mqtt_user" "")
MQTT_PASS=$(get_opt "mqtt_password" "")
MQTT_PREFIX=$(get_opt "mqtt_prefix" "enedis")

SYNC_INTERVAL_MINS=$(get_opt "sync_interval_mins" "60")
ENABLE_WEB_API=$(get_opt "enable_web_api" "true")
API_KEY=$(get_opt "api_key" "")

DB_PATH="/data/enedis.db"
DB_URL="sqlite://${DB_PATH}?mode=rwc"

export DATABASE_URL="$DB_URL"

if [ -n "$DC_CLIENT_ID" ]; then
    export DATA_CONNECT_CLIENT_ID="$DC_CLIENT_ID"
fi
if [ -n "$DC_CLIENT_SECRET" ]; then
    export DATA_CONNECT_CLIENT_SECRET="$DC_CLIENT_SECRET"
fi

echo "📋 Configuration détectée :"
echo " - PRM Linky                : ${PRM:-Non configuré}"
echo " - Fournisseur Enedis       : ${PROVIDER}"
echo " - Base de données locale   : ${DB_PATH}"
echo " - Hôte MQTT                : ${MQTT_HOST}:${MQTT_PORT} (préfixe: ${MQTT_PREFIX})"
echo " - Cycle de synchronisation : ${SYNC_INTERVAL_MINS} minutes"
echo " - Serveur REST / Ingress   : ${ENABLE_WEB_API} (port 8080)"

# Construction de l'URL MQTT
MQTT_AUTH=""
if [ -n "$MQTT_USER" ] && [ -n "$MQTT_PASS" ]; then
    MQTT_AUTH="${MQTT_USER}:${MQTT_PASS}@"
elif [ -n "$MQTT_USER" ]; then
    MQTT_AUTH="${MQTT_USER}@"
fi
MQTT_BROKER_URL="mqtt://${MQTT_AUTH}${MQTT_HOST}:${MQTT_PORT}"

# Publication de la découverte Home Assistant initiale si le PRM est spécifié
if [ -n "$PRM" ] && [ "$PRM" != "01234567890123" ]; then
    echo "📡 Enregistrement initial des capteurs Home Assistant Discovery via MQTT..."
    enedis mqtt publish \
        --broker "$MQTT_BROKER_URL" \
        --prefix "$MQTT_PREFIX" \
        --prm "$PRM" \
        --db "$DB_URL" || echo "⚠️ Enregistrement Discovery différé (broker MQTT en attente de démarrage)."
fi

# Calcul de l'intervalle en secondes
INTERVAL_SECS=$((SYNC_INTERVAL_MINS * 60))
if [ "$INTERVAL_SECS" -lt 300 ]; then
    INTERVAL_SECS=300 # Minimum 5 minutes pour respecter les quotas API Enedis
fi

AGENT_PID=""
stop_all() {
    echo "🛑 Signal d'arrêt reçu, arrêt des services Enedis..."
    if [ -n "$AGENT_PID" ]; then
        kill -TERM "$AGENT_PID" 2>/dev/null || true
    fi
    exit 0
}
trap stop_all SIGTERM SIGINT

# Lancement de l'agent de collecte continue en arrière-plan
echo "🚀 Démarrage du démon de collecte automatique (cycle: ${INTERVAL_SECS}s)..."
enedis agent \
    --db "$DB_URL" \
    --interval-secs "$INTERVAL_SECS" \
    --mqtt-broker "$MQTT_BROKER_URL" \
    --mqtt-prefix "$MQTT_PREFIX" &
AGENT_PID=$!

# Lancement du serveur Web API REST / Swagger UI en avant-plan
if [ "$ENABLE_WEB_API" = "true" ]; then
    API_ARGS=(serve --host 0.0.0.0 --port 8080 --db "$DB_URL")
    if [ -n "$API_KEY" ]; then
        API_ARGS+=(--api-key "$API_KEY")
    fi
    echo "🌐 Lancement du serveur Web API sur http://0.0.0.0:8080..."
    exec enedis "${API_ARGS[@]}"
else
    echo "ℹ️ Serveur Web désactivé, surveillance de l'agent de collecte..."
    wait "$AGENT_PID"
fi
