# 🚀 Déploiement Complet enedis-rs (Docker Compose)

Ce répertoire fournit un environnement de production complet prêt à l'emploi (**Out-of-the-box**) en une seule commande, comprenant :

| Service | Rôle | Port | Accès |
|---|---|---|---|
| **`enedis-api`** | Serveur API HTTP REST & Export Prometheus (sécurisable par clé d'API) | `8080` | [http://localhost:8080](http://localhost:8080) |
| **`enedis-agent`** | Daemon de collecte continue avec Rate Limiter & Publication MQTT | - | Logs Docker |
| **`mosquitto`** | Broker MQTT pour intégration Home Assistant Auto-Discovery | `1883` | `mqtt://localhost:1883` |
| **`grafana`** | Tableaux de bord pré-configurés (Courbes de charge & Santé SGE) | `3000` | [http://localhost:3000](http://localhost:3000) (admin / admin) |
| **`prometheus`** | Scrape automatique des métriques toutes les 10s | `9091` | [http://localhost:9091](http://localhost:9091) |
| **`postgres`** | Base temporelle PostgreSQL 16 (Schémas + Données démo 7j) | `5432` | `postgres://enedis:enedis_password@localhost:5432/enedis_db` |
| **`enedis-mock`** | Simulateur Enedis (SGE SOAP mTLS & Data Connect REST v5) | `9090` | [http://localhost:9090/services/](http://localhost:9090/services/) |

---

## ⚡ Démarrage Rapide (En 5 minutes chrono)

### 1. Lancer l'infrastructure complète
Depuis ce répertoire (`examples/docker-compose/`) :

```bash
docker compose up -d
```

> **Note :** La base PostgreSQL s'initialise automatiquement avec le schéma requis et **7 jours de courbes de charge réalistes** (un profil résidentiel standard `01234567890123` et un profil prosumer résidentiel avec injection solaire `09876543210987`).

### 2. Consulter le Dashboard Grafana
1. Ouvrez votre navigateur sur **[http://localhost:3000](http://localhost:3000)**.
2. Connectez-vous avec les identifiants :
   - **Utilisateur :** `admin`
   - **Mot de passe :** `admin`
3. Le tableau de bord **"Enedis - Suivi & Courbes de Charge"** s'affiche immédiatement.

### 3. Intégration Home Assistant (MQTT)
Le service Mosquitto reçoit automatiquement les messages **Home Assistant MQTT Discovery** publiés par `enedis-agent` dès le premier cycle de collecte :
- Topic de découverte : `homeassistant/sensor/enedis_<prm>_consumption/config`
- Topic d'état : `enedis/<prm>/state`
- Topic de disponibilité (LWT) : `enedis/status` (`online` / `offline`)

---

## 🔍 Tester l'API REST en Ligne de Commande

Vérifier l'état de l'API :
```bash
curl http://localhost:8080/health
# {"status":"UP","version":"0.1.0"}
```

Consulter les métriques Prometheus exportées :
```bash
curl http://localhost:8080/metrics
```

Si une clé d'API a été configurée via `ENEDIS_API_KEY` dans le fichier `.env` :
```bash
curl -H "X-API-Key: votre_cle_secrete" http://localhost:8080/api/v1/points
# ou
curl -H "Authorization: Bearer votre_cle_secrete" http://localhost:8080/api/v1/points
```

Récupérer les mesures de courbe de charge (format JSON structuré) :
```bash
curl "http://localhost:8080/api/v1/points/01234567890123/measurements"
```

Déclencher une synchronisation immédiate à la demande :
```bash
curl -X POST http://localhost:8080/api/v1/points/01234567890123/sync
```

---

## 🔐 Basculer en Production

Par défaut, la stack utilise le simulateur local `enedis-mock`. Pour basculer sur les serveurs réels d'Enedis :

### Mode SGE (SOAP mTLS)
1. Déposez votre certificat client mTLS PKCS#12 (`.p12`) dans `certs/` :
   ```bash
   mkdir -p certs
   cp /chemin/vers/mon_certificat.p12 certs/certificat.p12
   ```
2. Créez et éditez votre `.env` :
   ```dotenv
   ENEDIS_PROVIDER=sge
   ENEDIS_ENDPOINT=https://sge-services.enedis.fr/services/
   ENEDIS_CERT_P12=/etc/enedis/certs/certificat.p12
   ENEDIS_CERT_PASSWORD=votre_mot_de_passe_certificat
   ```

### Mode Data Connect (REST OAuth2)
Éditez votre `.env` :
```dotenv
ENEDIS_PROVIDER=dataconnect
ENEDIS_DATA_CONNECT_URL=https://ext.prod.api.enedis.fr
ENEDIS_DATA_CONNECT_CLIENT_ID=votre_client_id
ENEDIS_DATA_CONNECT_CLIENT_SECRET=votre_client_secret
```

Relancez les conteneurs :
```bash
docker compose up -d --force-recreate enedis-api enedis-agent
```

---

## 🛑 Arrêt de la Stack

Pour stopper l'ensemble des services :
```bash
docker compose down
```

Pour supprimer également les volumes persistants (remise à zéro complète) :
```bash
docker compose down -v
```
