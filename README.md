# ⚡ enedis-rs

**La passerelle Linky open-source, moderne, souveraine et sans abonnement.**

Suivez votre consommation d'électricité, vérifiez si votre abonnement Enedis est surdimensionné, comparez les offres (Base, Heures Creuses, Tempo) et intégrez votre compteur Linky en 1 clic dans **Home Assistant** ou sur un **tableau de bord web clé en main**.

- 🔒 **100% Souverain & Privé** : Vos données restent stockées chez vous (base locale SQLite ou PostgreSQL). Zéro publicité, zéro revente de données.
- 🏠 **Home Assistant en 1 Clic** : Add-on officiel prêt à l'emploi et 12 capteurs configurés automatiquement (consommation, injection solaire, puissance instantanée, alertes).
- 💻 **Tableau de Bord Web Inclus** : Visualisez vos graphiques et alertes directement dans votre navigateur sur `http://localhost:8080`.
- 💶 **Économisez sur vos factures** : Audit automatique d'abonnement (suis-je surdimensionné en 9 kVA ?) et simulateur d'économies tarifaires (Base vs Heures Creuses vs EDF Tempo).
- ⚡ **Compatible Enedis Officiel** : Connexion sécurisée en 1 clic via Enedis Data Connect v5 (OAuth2 officiel) ou Web Services SGE (mTLS).

---

## 🚀 Démarrage Rapide (En 3 minutes chrono)

Vous n'avez pas besoin d'être développeur ni d'installer Rust pour utiliser `enedis-rs` ! Choisissez la méthode adaptée à votre usage :

### Option 1 — Avec Home Assistant (Le plus simple pour la maison)
Idéal si vous avez Home Assistant sur Raspberry Pi ou mini-PC :
1. Dans Home Assistant, rendez-vous dans **Paramètres** > **Modules complémentaires** > **Boutique de modules complémentaires**.
2. Cliquez sur les trois points en haut à droite > **Dépôts**, et ajoutez l'URL de ce dépôt Git.
3. Installez le module **Enedis Linky Gateway**, renseignez votre numéro de compteur (PRM à 14 chiffres) et cliquez sur **Démarrer**.
4. Vos 12 capteurs apparaissent immédiatement dans votre **Tableau de bord Énergie (Energy Dashboard)** !
👉 [Consulter le guide détaillé Home Assistant](homeassistant-addon/DOCS.md)

### Option 2 — Avec Docker (En 1 commande)
Idéal si vous avez Docker sur votre NAS (Synology, QNAP, Unraid) ou votre serveur personnel :
```bash
git clone https://github.com/mat/enedis-rs.git
cd enedis-rs/examples/docker-compose
docker compose up -d
```
Ouvrez ensuite votre navigateur sur **`http://localhost:8080`** :
- Le tableau de bord web s'affiche immédiatement avec des données d'exemple pour tester.
- Cliquez sur **"+ Connecter un Linky"** pour lier votre vrai compteur en toute sécurité via le portail officiel Enedis.

### Option 3 — En Ligne de Commande (Binaire autonome prêt à l'emploi)
Téléchargez simplement le binaire précompilé pour votre système (Linux / macOS / Raspberry Pi) depuis les [Releases](https://github.com/mat/enedis-rs/releases), puis lancez :
```bash
# Vérifier la configuration en 1 seconde
./enedis doctor

# Démarrer le serveur et le tableau de bord web
./enedis serve --port 8080
```

---

## 🔌 Comment connecter votre compteur Linky ?

La synchronisation se fait en 3 étapes simples :
1. **Lancez l'application** (via Home Assistant, Docker ou le binaire).
2. **Ouvrez le tableau de bord** (`http://localhost:8080`) et cliquez sur le bouton **"+ Connecter un Linky"**.
3. **Validez sur le portail sécurisé d'Enedis** : vous vous identifiez sur votre compte Enedis habituel et validez l'accès à vos mesures.
4. Vos courbes de charge de consommation (et d'injection si vous avez des panneaux solaires) sont désormais automatiquement synchronisées chaque jour !

*(Pour essayer immédiatement sans compte Enedis, un simulateur local est inclus par défaut).*

> [!NOTE]
> **Fréquence et latence des données Linky :**
> Conformément au fonctionnement du gestionnaire de réseau Enedis, les courbes de charge de la veille (J-1) sont consolidées et publiées chaque matin vers **06h00 UTC**. Cette application interroge les serveurs officiels Enedis et n'a pas vocation à remplacer la télé-information client (TIC) temps réel à la seconde.
> 
> **Particuliers vs Professionnels :**
> - **Particuliers & Home Assistant** : Utilisez le mode **Enedis Data Connect v5** (connexion en 1 clic avec votre compte client Enedis).
> - **Acteurs de marché (B2B)** : Le mode **SGE Web Services (mTLS SOAP)** est également supporté pour les entités disposant d'un certificat client X.509 délivré par la PKI Enedis.

---

## 💡 Ce que vous pouvez faire au quotidien avec `enedis-rs`

| Fonctionnalité | Ce que cela vous apporte | Comment l'utiliser |
|---|---|---|
| 📊 **Tableau de bord web** | Vos courbes de consommation jour/semaine/mois avec tooltip au survol et bandeau d'alertes en temps réel. | Ouvrez `http://localhost:8080` dans votre navigateur. |
| 🏠 **Home Assistant (12 capteurs)** | Énergie soutirée, solaire injecté, puissance instantanée, pointe du jour, talon de veille, prix spot, le tout intégré dans Energy Dashboard. | [Guide Add-on](homeassistant-addon/DOCS.md) |
| 🔍 **Audit d'abonnement (kVA)** | Vérifie si vous payez pour un abonnement trop élevé (ex: 9 kVA alors que vous ne dépassez jamais 5 kVA) ou si vous risquez de disjoncter. | Bouton dans le tableau de bord ou `enedis max-power --audit` |
| 💶 **Comparateur de tarifs (€)** | Calcule votre facture réelle et simule vos économies si vous passiez à l'option Heures Creuses ou EDF Tempo. | `enedis costs --tariff tempo` |
| ⚡ **Alertes EcoWatt & Tempo** | Affiche la couleur Tempo du lendemain (Bleu/Blanc/Rouge) et le niveau de tension du réseau électrique RTE EcoWatt (Vert/Orange/Rouge). | Visible sur le tableau de bord ou `enedis signals tempo` |
| 📈 **Marché Spot (EPEX SPOT)** | Suivez les cours horaires du marché de l'électricité et calculez le potentiel d'arbitrage en déplaçant vos consommations modulables. | `enedis spot arbitrage --prm <PRM>` |
| 🩺 **Diagnostic automatique** | Teste en 1 seconde votre connexion, vos identifiants et votre base de données. | `enedis doctor` |
| 💾 **Export de données** | Téléchargez vos données pour Excel ou vos outils favoris (CSV, JSON, InfluxDB, Parquet, DuckDB). | `enedis measurements export --format csv` |

---

## 🚀 Déploiement en 1 Commande (Docker Compose & Grafana)

Lancez l'ensemble de l'infrastructure (`enedis-api`, `enedis-agent`, `postgres`, `prometheus`, `grafana`, `enedis-mock`) pré-configurée avec 7 jours de courbes de charge réalistes :

```bash
cd examples/docker-compose
docker compose up -d
```

- **Dashboard Grafana clé en main :** [http://localhost:3000](http://localhost:3000) (`admin` / `admin`)
- **API REST & Métriques :** [http://localhost:8080](http://localhost:8080)
- **Prometheus :** [http://localhost:9091](http://localhost:9091)
- **Simulateur SGE :** [http://localhost:9090](http://localhost:9090)

> [!TIP]
> Par défaut, ce déploiement Docker utilise le simulateur intégré `enedis-mock` afin de démarrer immédiatement sans certificat réel. Pour basculer vers les serveurs de production Enedis, montez votre fichier `.p12` dans `certs/` et ajustez les variables dans `.env`.

Consultez le guide détaillé dans [examples/docker-compose/README.md](examples/docker-compose/README.md).

## 🗄️ Stockage Temporel & Mutabilité de la donnée

La donnée Enedis n'est pas immuable : un pas de mesure peut d'abord être **Estimé**, puis **Reconstitué**, et enfin **Validé / Mesuré**.

### Règle d'or de l'UPSERT
Une ligne existante n'est mise à jour **que si la nouvelle qualité est supérieure ou égale** :
$$\text{Mesuré (3)} \ge \text{Reconstitué (2)} \ge \text{Estimé (1)}$$

```sql
INSERT INTO measurements (
    point_id, timestamp, direction, interval_seconds, value, unit, quality
) VALUES ($1, $2, $3, $4, $5, $6, $7)
ON CONFLICT (point_id, timestamp, direction) DO UPDATE SET
    value = EXCLUDED.value,
    interval_seconds = EXCLUDED.interval_seconds,
    unit = EXCLUDED.unit,
    quality = EXCLUDED.quality,
    updated_at = CURRENT_TIMESTAMP
WHERE EXCLUDED.quality >= measurements.quality;
```

### ⚡ Optimisation Bulk UPSERT par lots
Pour ingérer de gros volumes de données historiques (ex: 1 an = 17 520 points 30 min) sans saturer les allers-retours réseau :
- **SQLite** : Découpage automatique en lots de 100 mesures (700 paramètres < limite SQLite 999).
- **PostgreSQL** : Découpage en lots de 500 mesures avec `QueryBuilder`, divisant le temps d'ingestion par plus de 50x.

### ⏱️ Partitionnement Temporel & TimescaleDB
- **TimescaleDB** : Détection automatique de l'extension `timescaledb` lors de `init_schema()`, convertissant automatiquement la table `measurements` en hypertable avec des chunks temporels mensuels (`INTERVAL '1 month'`).
- **Partitionnement Déclaratif Natif** : Méthode `init_native_partitioned_schema()` et gestionnaire de partitions mensuelles `ensure_monthly_partition(year, month)` avec partition par défaut `measurements_default`.

### 📊 Moteur d'Agrégation Temporelle & Métrologie
Le module de calculs métrologiques (`aggregate_measurements`) permet de consolider à la volée les séries 30 minutes vers des granularités supérieures :
- **Intervalles supportés** : `hour`, `day`, `month`, `year`.
- **Indicateurs calculés** : Énergie totale cumulée (`total_energy_kwh`), puissances maximale, minimale et moyenne (`max_power_w`, `min_power_w`, `avg_power_w`), et nombre d'échantillons (`sample_count`).
- **Accessibilité** : Disponible via l'API REST (`/aggregates`), la CLI (`export --interval`) et le trait `StorageBackend`.

---

## 🌐 API HTTP REST, Swagger UI & Observabilité Prometheus

Lancez le serveur d'API intégré (avec choix du fournisseur et protection optionnelle par clé d'API) :
```bash
# Avec Enedis SGE (défaut)
cargo run --features cli,api,storage-sqlite --bin enedis -- serve --port 8080 --api-key "votre_cle_secrete"

# Ou avec Enedis Data Connect
cargo run --features cli,api,storage-sqlite --bin enedis -- \
    --provider dataconnect \
    --dc-url https://ext.prod.api.enedis.fr \
    --dc-client-id "mon_client_id" \
    --dc-client-secret "mon_secret" \
    serve --port 8080 --api-key "votre_cle_secrete"
```

### 🔒 Sécurité & Authentification API Key
Lorsque l'option `--api-key` (ou variable d'environnement `ENEDIS_API_KEY`) est définie :
- Tous les endpoints `/api/v1/*` exigent une authentification valide.
- Deux formats d'en-tête sont acceptés au choix :
  - `X-API-Key: votre_cle_secrete`
  - `Authorization: Bearer votre_cle_secrete`
- **Comparaison en temps constant** : La vérification utilise [`subtle::ConstantTimeEq`](https://docs.rs/subtle) pour prémunir le serveur contre toute attaque temporelle par canal auxiliaire (side-channel timing attack).
- Les requêtes sans clé ou avec clé incorrecte sont immédiatement rejetées en `401 Unauthorized`.
- Les sondes `/health` et `/metrics`, ainsi que la documentation OpenAPI/Swagger UI, restent ouvertes pour les orchestrateurs (Kubernetes, Docker) et outils de supervision.

### 🖥️ Tableau de Bord Web Moderne Clé en Main (Single-Page App)
Rendez-vous simplement sur **`http://localhost:8080/`** (ou `/dashboard`) dans votre navigateur :
- **Supervision multi-PRMs en temps réel** : Liste interactive des points de livraison gérés, statut de synchronisation et fraîcheur métrologique.
- **Gestion intégrée de la Clé API** : Boîte de dialogue de saisie dans l'en-tête, persistance dans le `localStorage` du navigateur, injection transparente des en-têtes `X-API-Key` sur tous les appels REST et détection interactive du code HTTP 401.
- **Visualisation graphique dynamique (Canvas)** : Courbes de charge interactives avec sélecteur temporel (7, 14, 30 jours), repères et infobulles.
- **Télémesure réseau en direct** : Alertes de tension réseau RTE EcoWatt et calendrier EDF Tempo (J / J+1).
- **Accès instantané aux Audits & Synchronisations** : Déclenchement de collectes à la demande en 1 clic et consultation directe du rapport d'audit d'optimisation financière.
- **Tunnel d'activation Linky** : Bouton d'onboarding direct et copie d'URL d'invitation client pour Enedis Data Connect v5.

### Documentation OpenAPI 3.0 & Swagger UI
- **Tableau de bord Web interactif** : `http://localhost:8080/`
- **Swagger UI interactif** : `http://localhost:8080/swagger-ui/`
- **Spécification OpenAPI 3.0 (JSON brut)** : `http://localhost:8080/api-docs/openapi.json`

### Endpoints disponibles
- `GET /` & `GET /dashboard` : Tableau de bord web interactif complet (SPA autonome)
- `GET /health` : Liveness & Readiness probe (`{"status":"UP","version":"0.1.0"}`)
- `GET /metrics` : Métriques au format Prometheus (`text/plain; version=0.0.4`) :
  - `enedis_requests_total{status, direction}`
  - `enedis_sync_errors_total{code, point_id}`
  - `enedis_collector_last_sync_timestamp{point_id, direction}`
  - `enedis_collector_data_freshness_seconds{point_id, direction}`
- `GET /consent/authorize` : Redirection OAuth2 vers le portail Enedis avec jeton CSRF cryptographique (CSPRNG 256 bits)
- `GET /consent/callback` : Réception du code d'autorisation Enedis et finalisation de l'onboarding
- `GET /api/v1/consent/url` : Génération JSON de l'URL d'autorisation OAuth2 pour intégrations front-end
- `POST /api/v1/consent/exchange` : Échange programmatique de jeton OAuth2 et persistance immédiate du PRM
- `GET /api/v1/points` : Liste de tous les PRMs et de leur statut
- `GET /api/v1/points/:prm` : Détails de synchronisation (Consommation & Production)
- `GET /api/v1/points/:prm/measurements?from=...&to=...&direction=...&limit=...` : Mesures normalisées en JSON (plafond par défaut de 20 000 points prévenant tout déni de service mémoire)
- `GET /api/v1/points/:prm/aggregates?from=...&to=...&interval=...&direction=...` : Calcul des agrégations temporelles (`hour`, `day`, `month`, `year`)
- `POST /api/v1/points/:prm/sync` : Déclenchement d'une collecte immédiate à la demande
- `GET /api/v1/points/:prm/costs?from=...&to=...&tariff_type=base|tempo|hphc|dynamic` : Estimation de la facture énergétique en Euros (€), calcul des taxes et comparatif
- `POST /api/v1/points/:prm/costs` : Calcul financier avec grille tarifaire personnalisée (JSON)
- `GET /api/v1/signals/tempo` : État et couleur du jour et du lendemain (Bleu, Blanc, Rouge)
- `GET /api/v1/signals/ecowatt` : Signaux de tension horaire du réseau électrique RTE EcoWatt (Vert, Orange, Rouge)
- `GET /api/v1/spot/prices?from=...&to=...&limit=...` : Historique et cours horaires du marché spot Day-Ahead (EPEX SPOT France)
- `GET /api/v1/points/:prm/spot/analysis?from=...&to=...&margin=...` : Corrélation de la courbe de charge avec les cours spot, coefficient de profilage et potentiel d'arbitrage (€/an)

---

## 💶 Moteur Tarifaire & Calcul des Coûts (€)

Le module de valorisation financière permet de convertir les courbes de charge de consommation en factures énergétiques réelles ou estimées :

- **Option Base** : Valorisation à tarif unique du kWh avec abonnement fixe mensuel.
- **Option Heures Pleines / Heures Creuses (HP/HC)** : Prise en charge des créneaux horaires locaux configurables (ex: 22h-06h, créneaux fractionnés type 12h-14h et 01h-07h).
- **Option EDF Tempo** : Prise en charge des 6 tarifs (Bleu/Blanc/Rouge × HP/HC) avec application stricte de la bascule horaire Tempo à 06h00 locale.
- **Offres Dynamiques** : Tarification au pas horaire indexée sur les cours du marché (prix spot horaire + marge fournisseur).
- **Décomposition complète** : Part consommation HT, abonnement au prorata de la période, taxes françaises (TVA 5.5% et 20%, accise/TICFE), total TTC et coût moyen pondéré du kWh.
- **Comparateur intégré** : Calcule automatiquement les économies ou surcoûts par rapport au tarif réglementé Base.

```bash
# Exemple d'estimation financière en ligne de commande
enedis costs --prm 01234567890123 \
    --from 2026-09-01T00:00:00Z \
    --to 2026-09-28T00:00:00Z \
    --tariff tempo
```

---

## 📈 Marché Spot Day-Ahead & Tarification Dynamique (EPEX SPOT)

Le module `spot` permet d'analyser l'adéquation entre la courbe de consommation réelle du foyer et les cours horaires du marché de gros de l'électricité (EPEX SPOT France / Day-Ahead) :

### 1. Indicateurs de Marché & Profilage Linky
- **Prix Moyen Pondéré Linky (€/MWh)** : Coût d'approvisionnement réel de l'énergie pour le profil du client ($\frac{\sum P_{spot} \times kWh}{\sum kWh}$).
- **Coefficient de Profilage** : Ratio entre le prix moyen pondéré du foyer et la moyenne arithmétique brute du marché.
  - `< 1.0` : Foyer vertueux et effacé (recharge en heures solaires ou de nuit).
  - `> 1.0` : Foyer accentuant la pointe de consommation lors des tensions de réseau.
- **Détection des Prix Négatifs** : Identification des heures où le prix de gros passe sous 0 €/MWh (abondance d'énergie renouvelable éolienne/solaire le week-end) et comptabilisation des kWh consommés durant ces heures.
- **Potentiel d'Arbitrage (€/an)** : Calcul de l'économie financière réalisable en déplaçant les charges modulables (recharge de véhicule électrique, ballon d'eau chaude) sur les heures de prix spot minimums.

```bash
# Consultation des cours horaires spot Day-Ahead (avec synchronisation en ligne)
enedis spot prices --sync --limit 48

# Analyse du profil Linky et calcul du potentiel d'arbitrage
enedis spot arbitrage --prm 01234567890123

# Export JSON
enedis spot arbitrage --prm 01234567890123 --format json
```

---

## ⚡ Signaux Réseau (RTE EcoWatt & EDF Tempo)

Connecteur et moteur d'alignement avec les signaux de tension du réseau électrique français :

- **Connecteur Open Data RTE EcoWatt & Tempo** : Collecte automatique de la couleur Tempo du jour (J) et du lendemain (J+1) et des signaux de tension EcoWatt (Vert / Orange / Rouge).
- **Persistance dédiée** : Tables `tempo_days` et `ecowatt_signals` avec gestion des conflits et requêtage temporel rapide (PostgreSQL & SQLite).
- **Indicateurs de flexibilité & corrélation** :
  - **Score d'effacement en pic (% de réduction de puissance)** en période rouge par rapport à la consommation de référence.
  - **Part de l'énergie consommée en période de tension** (Orange / Rouge).
  - **Ratio de report en Heures Creuses** lors des jours rouges Tempo.

```bash
# Synchronisation et consultation des signaux réseau
enedis signals tempo --sync
enedis signals ecowatt --sync

# Analyse de corrélation de la consommation d'un PRM
enedis signals correlation --prm 01234567890123 \
    --from 2026-01-01T00:00:00Z \
    --to 2026-01-31T23:59:59Z
```


---

## 🏠 Intégration Home Assistant & Energy Dashboard (MQTT Auto-Discovery)

Le module optionnel `mqtt` (piloté par le client asynchrone `rumqttc`) permet d'injecter automatiquement les données Enedis ainsi que des capteurs de puissance et de diagnostic dans Home Assistant et son **Energy Dashboard** via le protocole standard **MQTT Auto-Discovery**.

### 🚀 Lancement de l'Agent de collecte avec publication MQTT
```bash
cargo run --features cli,agent,mqtt,storage-sqlite --bin enedis -- agent \
    --interval-secs 3600 \
    --mqtt-broker mqtt://192.168.1.50:1883 \
    --mqtt-prefix enedis \
    --mqtt-user homeassistant \
    --mqtt-password mon_mot_de_passe
```

### 📡 Publication immédiate à la demande (CLI)
Pour tester ou forcer la publication des configurations Home Assistant et de l'état consolidé sans attendre le daemon :
```bash
cargo run --features cli,mqtt,storage-sqlite --bin enedis -- point mqtt-publish \
    --prm 01234567890123 \
    --mqtt-broker mqtt://192.168.1.50:1883 \
    --mqtt-prefix enedis
```

### 📊 12 Capteurs auto-découverts dans Home Assistant
Tous les capteurs sont automatiquement rattachés à un équipement virtuel unique `"Compteur Linky"` (`Compteur {PRM}`) avec version logicielle et topic de disponibilité LWT :
1. **Énergie Consommée** (`homeassistant/sensor/enedis_{prm}_consumption/config`) :
   - `device_class`: `energy`, `state_class`: `total_increasing`, `unit_of_measurement`: `kWh`
   - Intégrable immédiatement dans le panneau **Energy Dashboard** de Home Assistant.
2. **Énergie Produite** (`homeassistant/sensor/enedis_{prm}_production/config`) :
   - `device_class`: `energy`, `state_class`: `total_increasing`, `unit_of_measurement`: `kWh`
3. **Puissance Active Instantanée** (`homeassistant/sensor/enedis_{prm}_power/config`) :
   - `device_class`: `power`, `state_class`: `measurement`, `unit_of_measurement`: `W`
4. **Pointe Maximale Quotidienne** (`homeassistant/sensor/enedis_{prm}_max_power/config`) :
   - `device_class`: `apparent_power`, `state_class`: `measurement`, `unit_of_measurement`: `kVA`
5. **Puissance Souscrite** (`homeassistant/sensor/enedis_{prm}_subscribed_power/config`) :
   - `device_class`: `apparent_power`, `entity_category`: `diagnostic`, `unit_of_measurement`: `kVA`
6. **Talon de Veille (Baseload)** (`homeassistant/sensor/enedis_{prm}_baseload/config`) :
   - `device_class`: `power`, `state_class`: `measurement`, `unit_of_measurement`: `W`
7. **Taux d'Autoconsommation Solaire** (`homeassistant/sensor/enedis_{prm}_autoconsumption/config`) :
   - `unit_of_measurement`: `%`, `state_class`: `measurement`
8. **Taux d'Autoproduction Solaire** (`homeassistant/sensor/enedis_{prm}_autoproduction/config`) :
   - `unit_of_measurement`: `%`, `state_class`: `measurement`
9. **Prix Spot Day-Ahead** (`homeassistant/sensor/enedis_{prm}_spot_price/config`) :
   - `unit_of_measurement`: `€/MWh`, `state_class`: `measurement`
10. **Qualité Métrologique** (`homeassistant/sensor/enedis_{prm}_quality/config`) :
   - `entity_category`: `diagnostic` (`VALIDATED`, `ESTIMATED`, `CORRECTED`)
11. **Horodatage du Dernier Relevé** (`homeassistant/sensor/enedis_{prm}_last_reading/config`) :
   - `device_class`: `timestamp`, `entity_category`: `diagnostic`
12. **Statut de Synchronisation** (`homeassistant/sensor/enedis_{prm}_sync_status/config`) :
   - `entity_category`: `diagnostic` (`OK`, `CONSENT_EXPIRED`, etc.)

### 🟢 Disponibilité & LWT (Last Will and Testament)
- Le statut du service est publié sur `enedis/status` (`online` au démarrage avec `retain: true`).
- En cas de coupure inopinée ou crash du daemon, le broker MQTT diffuse automatiquement `offline`, marquant les entités comme indisponibles dans Home Assistant.


---

## 🩺 `enedis doctor`

La commande centrale de diagnostic permet de débloquer les situations complexes liées à la bureaucratie Enedis :

```bash
cargo run --features cli,storage-sqlite --bin enedis -- doctor --prm 01234567890123
```

Exemple de rapport généré :
```text
🏥 === RAPPORT DE DIAGNOSTIC ENEDIS DOCTOR ===

  ✅ 1. Certificat d'authentification mTLS client
     Fichier PKCS#12 "certs/client.p12" valide et mot de passe vérifié avec succès.

  ✅ 2. Négociation mTLS & Connexion réseau SGE
     Connectivité et mTLS validés avec Enedis SGE.

  ⚠️  3. Vérification du consentement et des habilitations SGE
     Consentement manquant pour le PRM 01234567890123 : Le client doit valider le partage sur son espace Enedis.
     👉 Action corrective : Inviter l'usager à renouveler son accord.

  ✅ 4. Persistance & Base de données temporelle
     Base de données opérationnelle (12 PRM enregistrés pour synchronisation).
```

### 💻 Autres commandes utiles du CLI

```bash
# Téléchargement ponctuel et stockage en base
cargo run --features cli,storage-sqlite --bin enedis -- measurements fetch \
    --prm 01234567890123 \
    --from 2026-09-01T00:00:00Z \
    --to 2026-09-08T00:00:00Z \
    --direction consumption

# Export des agrégats journaliers au format CSV (ou JSON brut)
cargo run --features cli,storage-sqlite --bin enedis -- measurements export \
    --prm 01234567890123 \
    --format csv \
    --interval day

# Consultation des points de livraison gérés
cargo run --features cli,storage-sqlite --bin enedis -- point list

# Consultation des caractéristiques contractuelles (puissance souscrite, option tarifaire, compteur)
cargo run --features cli --bin enedis -- contract --prm 01234567890123

# Pointes maximales quotidiennes de puissance atteinte et audit d'abonnement
cargo run --features cli --bin enedis -- max-power \
    --prm 01234567890123 \
    --from 2026-09-01T00:00:00Z \
    --to 2026-09-29T00:00:00Z \
    --audit

# Suivi proactif du cycle de vie et alertes d'expiration du consentement
cargo run --features cli --bin enedis -- consent --prm 01234567890123 --warning-days 30

# Utilisation avec l'API REST Data Connect au lieu de SGE
cargo run --features cli --bin enedis -- \
    --provider dataconnect \
    --dc-token "mon_bearer_token" \
    contract --prm 01234567890123
```

---

## ⚡ Extension des Opérations SGE & Support Enedis Data Connect

`enedis-rs` propose une architecture unifiée pour adresser les deux modes d'échange Enedis majeurs :

1. **Web Services SGE (SOAP 1.1 + mTLS)** : Destinés aux acteurs de marché (fournisseurs, gestionnaires, agrégateurs) titulaires d'un contrat d'échange avec authentification par certificat matériel ou PKCS#12.
2. **API Data Connect (REST v5 + OAuth2)** : Destinée aux particuliers, plateformes SaaS et intégrations tierces plus légères nécessitant une authentification par jetons Bearer OAuth2.

### Trait d'Abstraction `EnedisProvider`

Le trait [`EnedisProvider`](src/client/provider.rs) abstrait intégralement la source de données pour permettre l'interchangeabilité dynamique :

```rust
use enedis_rs::{DataConnectClient, DataConnectConfig, EnedisProvider, FlowDirection, PointId, SgeClient, SgeClientConfig};
use std::sync::Arc;

async fn inspect_prm(provider: Arc<dyn EnedisProvider>, prm: PointId) -> Result<(), Box<dyn std::error::Error>> {
    println!("Source active : {}", provider.provider_name());

    // 1. Données contractuelles (kVA, Option tarifaire, Caractéristiques compteur)
    let contract = provider.fetch_contract_data(prm).await?;
    println!("Puissance : {} kVA, Option : {:?}", contract.subscribed_power_kva, contract.tariff_option);

    // 2. Pointes de puissance atteinte
    let from = chrono::Utc::now() - chrono::Duration::days(30);
    let to = chrono::Utc::now();
    let max_powers = provider.fetch_daily_max_power(prm, from, to).await?;

    // 3. Suivi du cycle de vie et alerte proactive
    let consent = provider.fetch_consent_status(prm).await?;
    let alert = consent.check_alert(30, chrono::Utc::now());
    if alert.severity != enedis_rs::ConsentAlertSeverity::Healthy {
        println!("⚠️ Attention : {}", alert.message);
    }

    Ok(())
}
```

### Opérations Métier Disponibles

- **`consulterDonneesContractuelles`** : Récupère la puissance souscrite ($kVA$), l'option tarifaire (`Base`, `HeuresPleinesCreuses`, `Tempo`, `Ejp`), le calendrier fournisseur (plages d'heures creuses, couleurs Tempo), et les caractéristiques techniques du compteur (matricule, Linky/CBE/électromécanique, monophasé/triphasé, réglage disjoncteur).
- **`consulterPuissanceMax` & Audit d'Abonnement** : Récupère l'historique des puissances maximales quotidiennes ($W$ / $kVA$). La fonction [`audit_subscription_sizing`](src/models/power.rs) compare ces pointes à la puissance souscrite et recommande le palier Enedis optimal ($3, 6, 9, 12, 15, 18, 24, 30, 36\text{ kVA}$) en détectant le surdimensionnement ou le risque de disjonction.
- **Suivi Proactif des Consentements** : Permet d'anticiper la rupture des flux en contrôlant la date d'échéance et en générant des alertes graduées (`Healthy`, `Warning`, `Critical`, `Expired`).

---

## ⚡ Agent Avancé, Rétention & Connecteurs Analytiques

### 1. Détection fine des "trous" (Gap Detection & Backfill)
L'agent ne se contente pas d'avancer séquentiellement : il inspecte la base pour cibler les jours manquants et déclencher des rattrapages chirurgicaux sans re-télécharger les périodes déjà présentes.

```bash
# Rattrapage chirurgical en ligne de commande
enedis measurements backfill --prm 01234567890123 \
    --from 2026-09-01T00:00:00Z \
    --to 2026-09-28T00:00:00Z \
    --direction consumption
```

### 2. Planification avancée (Cron)
Remplacez l'intervalle fixe en secondes par une expression Cron standard (ex: `0 4 * * *` pour lancer chaque nuit à 04h00 UTC, lorsque Enedis publie les courbes consolidées de la veille) :

```bash
# Lancement de l'agent en mode Cron
enedis agent --cron "0 4 * * *" --concurrency 4
```

### 3. Politique de rétention et compression (Rollup / Downsampling)
Pour les flottes de compteurs ou installations micro-serveurs (Raspberry Pi avec SQLite), archivez les pas de 30 minutes de plus de 2 ans sous forme d'agrégats journaliers ou horaires, libérant jusqu'à 98% d'espace disque avec défragmentation physique VACUUM automatique :

```bash
# Compactage manuel en ligne de commande
enedis measurements rollup --older-than-days 730 --interval daily --vacuum

# Ou activation automatique au sein du daemon de collecte :
enedis agent --cron "0 4 * * *" --retention-days 730 --rollup-interval daily
```

### 4. Connecteurs InfluxDB, Parquet & DuckDB
Exportez vos séries temporelles vers l'écosystème time-series et OLAP moderne :

```bash
# Export au format InfluxDB Line Protocol (fichier .lp ou stdout)
enedis measurements export --prm 01234567890123 --format influxdb --output enedis.lp

# Export Apache Parquet compressé (Snappy + indexation dictionnaire)
enedis measurements export --prm 01234567890123 --format parquet --output enedis.parquet

# Génération de script et vue analytique pour DuckDB
enedis measurements export --prm 01234567890123 --format duckdb --output analytics.sql
```

---

## 🔍 Audit de Puissance Souscrite & Évaluation Financière

La version open-source `enedis-rs` intègre nativement les calculs métrologiques et financiers pour auditer votre compteur Linky :

### 1. Audit de Dimensionnement d'Abonnement (kVA)
Détecte si la puissance souscrite est surdimensionnée (facture fixe trop élevée) ou sous-dimensionnée (risque de disjonction lors des pics d'hiver) en comparant les pointes atteintes aux paliers normalisés Enedis ($3, 6, 9, 12, 15, 18, 24, 30, 36\text{ kVA}$) :

```bash
# Analyse des pointes maximales quotidiennes et audit d'abonnement
enedis max-power --prm 01234567890123 \
    --from 2026-09-01T00:00:00Z \
    --to 2026-09-29T00:00:00Z \
    --audit
```

### 2. Évaluation des Coûts & Comparateur Tarifaire (€)
Valorise la consommation réelle au pas de 30 minutes selon la formule choisie (`base`, `hphc`, `tempo`, `dynamic`) avec calcul des taxes et simulation des économies annuelles par rapport au tarif réglementé :

```bash
# Simulation financière sur option EDF Tempo
enedis costs --prm 01234567890123 \
    --from 2026-09-01T00:00:00Z \
    --to 2026-09-28T00:00:00Z \
    --tariff tempo
```

---

## 💻 Tableau de Bord Web Embarqué (SPA Dark-Mode)

Un tableau de bord web interactif moderne et ultra-léger (zéro dépendance externe npm) est servi directement par Axum à la racine du serveur HTTP :

- **URL d'accès :** `http://localhost:8080/` ou `http://localhost:8080/dashboard`
- **Fonctionnalités intégrées :**
  - Bandeau télémétrique temps-réel (compteurs gérés, énergie totale mesurée, signal RTE EcoWatt, couleur EDF Tempo J et J+1).
  - Gestion sécurisée de la Clé API (saisie interactive, persistance locale, injection transparente sur toutes les requêtes REST et gestion automatique des 401).
  - Sélecteur multi-PRM avec métadonnées de dernière synchronisation.
  - Courbe de charge interactive (Canvas HTML5) avec filtres temporels (7, 14, 30 jours) et tooltip au survol.
  - Résumé instantané des gains annuels potentiels (€) et dimensionnement kVA.
  - Déclencheur de synchronisation immédiate et bouton de connexion Linky 1-clic.

---

## 🔐 Tunnel d'Onboarding & Consentement Automatisé (Enedis Data Connect OAuth2)

`enedis-rs` intègre un flux complet de recueil et d'activation automatisée du consentement client via Enedis Data Connect v5 OAuth2 :

### 1. Parcours Utilisateur
1. **Initiation :** L'utilisateur ou consultant clique sur **"+ Connecter un Linky"** dans le tableau de bord (ou ouvre `/consent/authorize`).
2. **Autorisation Enedis :** Redirection sécurisée (`302`/`303`) vers la page officielle Enedis OAuth2 avec jeton anti-CSRF cryptographique (CSPRNG 256 bits), durée (3 ans) et `client_id`.
3. **Validation :** L'usager s'identifie sur son compte Enedis et accorde le consentement sur son compteur.
4. **Callback & Enregistrement Automatique :** Enedis redirige vers `/consent/callback?code=...&usage_point_id=...`. Le serveur vérifie l'état CSRF, échange le code d'autorisation contre le jeton Bearer, enregistre immédiatement le PRM dans le stockage temporel, et affiche une page de confirmation avec retour au tableau de bord.

### 2. Endpoints du Tunnel
| Méthode | Route | Description |
|---|---|---|
| `GET` | `/consent/authorize` | Redirige le navigateur vers le portail d'autorisation Enedis OAuth2 avec état CSRF |
| `GET` | `/consent/callback` | Réceptionne le code OAuth2, valide le jeton CSRF, enregistre le PRM et affiche la confirmation |
| `GET` | `/api/v1/consent/url` | API JSON retournant l'URL d'autorisation et le jeton d'état pour intégrations front-end |
| `POST` | `/api/v1/consent/exchange` | Échange programmatique de code d'autorisation et enregistrement immédiat du PRM |

### 3. Invitation Client / Consultant Énergétique
Dans le tableau de bord, le bouton **"Copier le Lien d'Invitation Client"** génère instantanément l'URL publique de consentement partageable à vos clients par e-mail ou SMS pour un onboarding 100% sans friction.



---

## 🏠 Home Assistant : Add-on Officiel & Télémétrie MQTT Énergie

`enedis-rs` fournit une intégration officielle et prête à l'emploi pour Home Assistant :

### 1. Add-on Home Assistant (`homeassistant-addon/`)
- **Dépôt Supervisor officiel** : Déploiement en 1 clic dans Home Assistant OS / Supervised.
- **Support multi-architectures** : `amd64`, `aarch64` (Raspberry Pi 4/5, Home Assistant Yellow/Green), `armv7`.
- **Interface Ingress intégrée** : Accès direct au tableau de bord Web et à l'API depuis le menu latéral de Home Assistant.
- **Persistance locale** : Base de données SQLite stockée sous `/data/enedis.db`.

### 2. Auto-Discovery MQTT (12 Capteurs Linky)
Tous les capteurs sont automatiquement configurés et rattachés au composant **Home Assistant Energy Dashboard** :
- `sensor.enedis_{prm}_consumption` : Énergie soutirée cumulée (`total_increasing`, kWh).
- `sensor.enedis_{prm}_production` : Énergie injectée cumulée (solaire, kWh).
- `sensor.enedis_{prm}_power` : Puissance instantanée active (W).
- `sensor.enedis_{prm}_max_power` : Pointe de puissance du jour (kVA).
- `sensor.enedis_{prm}_subscribed_power` : Puissance contractuelle souscrite (kVA).
- `sensor.enedis_{prm}_baseload` : Talon de veille détecté (W).
- `sensor.enedis_{prm}_autoconsumption` : Taux d'autoconsommation solaire (%).
- `sensor.enedis_{prm}_autoproduction` : Taux d'autoproduction / indépendance solaire (%).
- `sensor.enedis_{prm}_spot_price` : Prix Day-Ahead EPEX SPOT en cours (€/MWh).
- Diagnostics : `sensor.enedis_{prm}_quality`, `last_reading`, `sync_status`.

Consultez le guide complet dans [homeassistant-addon/DOCS.md](homeassistant-addon/DOCS.md).

---

## 🛠️ Guide Développeur, Architecture & SDK Rust

Pour les développeurs souhaitant intégrer `enedis-rs` comme bibliothèque dans une application Rust ou comprendre l'architecture interne :

### Architecture Globale
```
┌─────────────────────────────────────────────────────────────────┐
│                   CLI / enedis doctor / API REST                │
└────────────────────────────────┬────────────────────────────────┘
                                 │
┌─────────────────────────┐      │        ┌─────────────────────────┐
│  Agent Daemon (Tokio)   ├──────┼───────►│  Storage (SQLx)         │
│  - Concurrence multi-PRM│      │        │  - PostgreSQL / SQLite  │
│  - Rate Limiter IP      │      │        │  - Bulk UPSERT par lots │
│  - Backfill chirurgical │      │        │  - TimescaleDB / Range  │
│  - Publication MQTT HA  │      │        │  - Compactage Rollup    │
└────────────┬────────────┘      │        └─────────────────────────┘
             │                   │
┌────────────▼───────────────────▼────────────────────────────────┐
│  EnedisProvider (Trait Unifié Asynchrone)                       │
│  ┌──────────────────────────────┬─────────────────────────────┐ │
│  │ SgeClient (SOAP mTLS)        │ DataConnectClient (OAuth2)  │ │
│  │ - mTLS PKCS#12 / PEM         │ - Bearer Token / OAuth2 v3  │ │
│  │ - quick-xml streaming 0-copy │ - REST JSON v5              │ │
│  │ - SOAP Fault & XML Sécurisé  │ - Renouvellement proactif   │ │
│  └──────────────────────────────┴─────────────────────────────┘ │
│  - Zéro Secret dans les logs (SecretString, Debug [REDACTED])   │
│  - Code 100% Safe Rust                                          │
└─────────────────────────────────────────────────────────────────┘
```

### Dépendance Cargo (`Cargo.toml`)
```toml
[dependencies]
enedis-rs = "0.1" # active par défaut le client HTTP mTLS et le moteur XML SOAP
tokio = { version = "1.0", features = ["full"] }
chrono = "0.4"
secrecy = "0.8"
```

*Pour activer également le stockage local (SQLite ou PostgreSQL) :*
```toml
enedis-rs = { version = "0.1", features = ["storage-sqlite"] } # ou storage-postgres
```

### Exemple minimal en Rust
```rust
use chrono::{Duration, Utc};
use enedis_rs::{
    ClientIdentitySource, FlowDirection, PointId, SgeClient, SgeClientConfig, SgeEnvironment,
};
use secrecy::SecretString;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = SgeClientConfig::with_environment(SgeEnvironment::Production);
    config.identity = Some(ClientIdentitySource::Pkcs12File {
        path: PathBuf::from("certs/client.p12"),
        password: SecretString::new("mon_mot_de_passe_certificat".to_string()),
    });

    let client = SgeClient::new(config)?;
    let prm = PointId::new("01234567890123")?;
    let to = Utc::now();
    let from = to - Duration::days(7);

    let measurements = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await?;

    println!("✅ Reçu {} pas de mesure :", measurements.len());
    for m in measurements.iter().take(5) {
        println!("  - {} : {} {} [{:?}]", m.timestamp, m.value, m.unit, m.quality);
    }

    Ok(())
}
```

### Feature Flags (`Cargo.toml`)
| Feature | Description | Dépendances activées |
|---|---|---|
| `client` *(default)* | Client HTTP mTLS, SOAP et gestion des credentials | `reqwest`, `tokio`, `secrecy`, `xml` |
| `xml` *(default)* | Sécurité XML, parseur streaming de courbes et enveloppes SOAP | `quick-xml` |
| `storage` | Couche d'abstraction générique de stockage SQL | `sqlx` |
| `storage-sqlite` | Persistance locale SQLite avec UPSERT conditionnel | `storage`, `sqlx/sqlite` |
| `storage-postgres` | Persistance PostgreSQL scalable pour grand parc de PRM | `storage`, `sqlx/postgres` |
| `agent` | Daemon de collecte continu avec rate limiting et supervision | `tokio`, `client`, `storage`, `croner` |
| `api` | Serveur API HTTP REST (endpoints OpenAPI/REST & Prometheus) | `axum`, `tokio`, `client`, `storage`, `openapi` |
| `openapi` | Spécification OpenAPI 3.0 et Swagger UI interactif | `utoipa`, `utoipa-swagger-ui` |
| `mqtt` | Auto-discovery et télémétrie pour Home Assistant & Energy Dashboard | `rumqttc`, `tokio`, `storage` |
| `cli` | Binaire CLI (`doctor`, `auth`, `point`, `measurements`, `agent`, `serve`, `mock`) | `clap`, `client`, `storage`, `tracing-subscriber` |
| `mock-sge` | Serveur mock local simulant les réponses SGE et pannes | `axum`, `tokio`, `client`, `xml` |
| `parquet` | Export et streaming haute performance Apache Parquet compressé | `parquet`, `arrow` |
| `analytics` | Connecteurs et formats d'exportation analytique OLAP (active `parquet`) | `parquet` |
| `wasm` | Moteur d'audit et simulation pour WebAssembly (navigateur / client-side) | `wasm-bindgen`, `serde-wasm-bindgen`, `console_error_panic_hook` |
| `native-tls-fallback`| Secours OpenSSL/SChannel si vieux chiffrements TLS Enedis | `reqwest/native-tls` |

---

## 🌐 Moteur WebAssembly (Wasm) & Démo Client-Side

Le moteur métrologique de `enedis-rs` est compilable en **WebAssembly**, permettant d'exécuter l'audit de dimensionnement de puissance Linky **directement dans le navigateur web du client**, sans envoyer de données personnelles à un serveur tiers (confidentialité RGPD absolue) :

### Fonctions Exportées Wasm (Community)
- `wasm_init()` : Initialise le gestionnaire de panique pour les traces détaillées dans la console JS.
- `wasm_audit_subscription(measurements_json, prm, subscribed_power_kva)` : Audit du dimensionnement de puissance souscrite ($3, 6, 9, 12, 15, 18, 24, 30, 36\text{ kVA}$), détection de disjonction ou de surdimensionnement et calcul des économies annuelles d'abonnement.


### Compilation et Démo
```bash
# Compilation WebAssembly (nécessite wasm-pack)
wasm-pack build --target web -- --features wasm --no-default-features

# Démonstrateur interactif
# Ouvrez directement wasm-demo/index.html dans votre navigateur !
```

---

## 🧪 Tests d'Intégration, Sécurité & Fuzzing

Le projet dispose d'une suite de **22 suites de tests d'intégration et unitaires** (100% passants via `cargo test --all-features`), d'une conformité stricte aux lints Rust (`cargo clippy --all-targets --all-features -- -D warnings`), d'un formatage irréprochable (`cargo fmt --check`) et d'un harnais de fuzzing complet :

- `agent_daemon_tests.rs` : Boucle de collecte, concurrence multi-PRM et mise en quarantaine automatique.
- `aggregation_storage_tests.rs` : Moteur d'agrégation temporelle (heure, jour, mois, année, calculs kWh et puissances W).
- `api_tests.rs` : Endpoints HTTP REST, authentification API Key en temps constant et métriques Prometheus.
- `consent_flow_tests.rs` : Validation end-to-end du tunnel OAuth2 Enedis (CSRF CSPRNG 256 bits, échange de code, enregistrement PRM).
- `cron_schedule_tests.rs` : Planification Cron, parsing et calcul des occurrences temporelles de relève.
- `data_connect_tests.rs` : Connecteur REST Enedis Data Connect v5, OAuth2 Bearer tokens et polymorphisme EnedisProvider.
- `e2e_mock_dataconnect_tests.rs` : Simulation OAuth2 Data Connect REST v5 et scénarios d'erreurs (consentement expiré, 404, quotas).
- `e2e_mock_sge_tests.rs` : Simulation de requêtes SOAP SGE, fautes réseau, retry avec exponential backoff.
- `error_classification_tests.rs` : Classification de résilience (retry, pause collector, quarantaine PRM) et explications doctor.
- `export_connectors_tests.rs` : Connecteurs InfluxDB Line Protocol, Apache Parquet compressé et scripts DuckDB (avec échappement SQL).
- `fuzz_simulation_tests.rs` : 2 500 mutations aléatoires garantissant zéro panic.
- `gap_detection_and_backfill_tests.rs` : Détection chirurgicale des trous et rattrapage automatique sans doublon.
- `mqtt_integration_tests.rs` : Auto-Discovery MQTT (12 capteurs, LWT) et publication des états pour Home Assistant Energy Dashboard.
- `retention_and_rollup_tests.rs` : Compactage downsampling des mesures anciennes et défragmentation VACUUM.
- `sge_extended_operations_tests.rs` : Données contractuelles, pointes de puissance max quotidienne et audit kVA.
- `sge_parser_tests.rs` : Parsing streaming de courbes de charge XML et ordonnancement de la qualité métrologique.
- `soap_tests.rs` : Validation d'encapsulation SOAP et détection des fautes avec contrôle de profondeur de balises.
- `spot_tests.rs` : Marché EPEX SPOT Day-Ahead, profils synthétiques réalistes, tarification dynamique et calculs d'arbitrage.
- `storage_upsert_tests.rs` : Preuve de l'UPSERT conditionnel (une estimation ne peut pas écraser une valeur certifiée).
- `tariff_and_signals_tests.rs` : Calcul des coûts d'énergie (Base, HP/HC, Tempo, Dynamique) et signaux RTE EcoWatt / EDF Tempo.
- `wasm_tests.rs` : Moteur WebAssembly in-browser (audit de dimensionnement d'abonnement).
- `xml_security_tests.rs` : Rejet des attaques Billion Laughs, DTD, bombes XML et dépassements de taille maximale `Content-Length`.
- `fuzz/` : Harnais standard `cargo-fuzz` / `libfuzzer` pour tests de sécurité continus.

---

## ❓ FAQ & Dépannage (Problèmes Fréquents)

<details>
<summary><b>1. Pourquoi mes données s'arrêtent-elles à hier ?</b></summary>
Le compteur Linky communique ses index à Enedis par CPL une fois par nuit. Enedis met les données à disposition le lendemain matin vers 06h00. Il est donc normal de ne pas avoir de données pour la journée en cours.
</details>

<details>
<summary><b>2. Erreur "Consentement introuvable ou expiré" (ADAM-ERR0069)</b></summary>
Les autorisations d'accès aux données Linky ont une durée de validité légale (généralement 1 à 3 ans). Cliquez simplement sur le bouton <b>"+ Connecter un Linky"</b> dans le tableau de bord web pour renouveler votre consentement en 30 secondes.
</details>

<details>
<summary><b>3. Erreur "Quota d'appels dépassé" (HTTP 429)</b></summary>
L'API Enedis impose des limites quotidiennes d'appels par point de livraison. Le daemon intégré <code>enedis-agent</code> gère automatiquement les délais d'attente (exponential backoff) et synchronise une seule fois par jour pour ne pas saturer votre quota.
</details>

<details>
<summary><b>4. Mon compteur n'est pas reconnu</b></summary>
Vérifiez que votre numéro de PRM (Point Référence Mesure) comporte exactement 14 chiffres (disponible sur l'écran de votre compteur Linky en faisant défiler avec la touche <code>+</code> ou sur votre facture d'électricité).
</details>

---

## 📜 Licence

Distribué sous double licence **MIT** ou **Apache-2.0** au choix.
Consultez les fichiers [LICENSE-MIT](LICENSE-MIT) et [LICENSE-APACHE](LICENSE-APACHE) pour l'intégralité des termes légaux.

