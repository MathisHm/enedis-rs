# Documentation de l'Add-on Enedis Linky Gateway pour Home Assistant

L'Add-on **Enedis Linky Gateway** permet d'intégrer nativement votre compteur communicant Linky dans **Home Assistant**, d'alimenter automatiquement le **Tableau de bord Énergie (Energy Dashboard)** et de bénéficier des analyses métrologiques avancées du projet `enedis-rs` (talon de veille, détection chauffe-eau, recharge véhicule électrique, solaire en autoconsommation et tarification dynamique spot).

---

## ⚡ Fonctionnalités clés

- 📡 **Auto-Discovery MQTT natif** : création automatique de tous les capteurs Linky dans Home Assistant sans aucune configuration manuelle de YAML.
- ⚡ **Compatible Home Assistant Energy** : capteurs de consommation et d'injection déclarés en `total_increasing` avec classe `energy` (kWh).
- 💧 **Désagrégation NILM & Veille** : surveillance permanente du talon de veille (`sensor.enedis_<prm>_baseload`) et détection des gros postes de charge.
- ☀️ **Bilan Solaire & Autoconsommation** : capteurs de taux d'autoconsommation et d'autoproduction (`sensor.enedis_<prm>_autoconsumption`, `sensor.enedis_<prm>_autoproduction`).
- 📈 **Marché Spot Day-Ahead** : remontée des cours horaires EPEX SPOT (`sensor.enedis_<prm>_spot_price`).
- 🌐 **Interface REST & Ingress** : interface Web Swagger UI et API HTTP accessible directement depuis la barre latérale de Home Assistant.

---

## 🚀 Installation & Configuration

### Étape 1 : Ajouter le dépôt d'Add-ons à Home Assistant
1. Dans Home Assistant, rendez-vous dans **Paramètres** > **Modules complémentaires** > **Boutique de modules complémentaires**.
2. Cliquez sur les **trois points verticaux** en haut à droite, puis sur **Dépôts**.
3. Ajoutez l'URL de votre dépôt Git `enedis-rs` (ou déposez le dossier `homeassistant-addon` dans votre répertoire `/addons/enedis_linky`).
4. Recherchez **Enedis Linky Gateway** et cliquez sur **Installer**.

### Étape 2 : Configurer les options du module
Dans l'onglet **Configuration** de l'Add-on :

| Paramètre | Description | Exemple |
|---|---|---|
| `prm` | Identifiant du Point de Livraison (14 chiffres) | `01234567890123` |
| `provider` | Fournisseur de collecte (`dataconnect`, `sge` ou `mock`) | `dataconnect` |
| `dataconnect_client_id` | Identifiant Client OAuth2 Enedis Data-Connect | `dc_client_xyz` |
| `dataconnect_client_secret` | Secret Client OAuth2 Enedis Data-Connect | `secret_12345` |
| `mqtt_host` | Hôte du broker MQTT (par défaut Mosquitto officiel) | `core-mosquitto` |
| `mqtt_port` | Port du broker MQTT | `1883` |
| `mqtt_user` | Utilisateur MQTT (laisser vide si broker interne avec anonyme) | `homeassistant` |
| `mqtt_password` | Mot de passe MQTT | `mot_de_passe` |
| `sync_interval_mins` | Intervalle entre chaque collecte en minutes (min: 5) | `60` |
| `enable_web_api` | Activer le serveur REST API & Ingress | `true` |

Cliquez sur **Enregistrer** puis démarrez le module complémentaire.

---

## 📊 Intégration dans le Tableau de Bord Énergie (Energy Dashboard)

Dès le démarrage, l'Add-on publie les configurations Discovery MQTT. Rendez-vous dans **Paramètres** > **Tableaux de bord** > **Énergie** :

### 1. Consommation du réseau (Grid consumption)
- Cliquez sur **Ajouter une consommation**.
- Sélectionnez le capteur : `sensor.enedis_<votre_prm>_consommation`.
- L'énergie cumulée sera synchronisée avec précision demi-horaire.

### 2. Retour au réseau / Production Solaire (Return to grid)
- Si vous disposez de panneaux photovoltaïques avec revente de surplus ou injection mesurée par Enedis, cliquez sur **Ajouter un retour**.
- Sélectionnez le capteur : `sensor.enedis_<votre_prm>_production`.

---

## 🔍 Entités créées automatiquement

| Entité Home Assistant | Unité | Description |
|---|---|---|
| `sensor.enedis_<prm>_consommation` | kWh | Consommation cumulée totale |
| `sensor.enedis_<prm>_production` | kWh | Injection solaire cumulée totale |
| `sensor.enedis_<prm>_puissance_active` | W | Puissance instantanée relevée |
| `sensor.enedis_<prm>_puissance_max` | kVA | Pointe de puissance maximale de la journée |
| `sensor.enedis_<prm>_puissance_souscrite` | kVA | Puissance souscrite contractuelle (diagnostic) |
| `sensor.enedis_<prm>_talon_de_veille` | W | Puissance talon absorbée en continu 24h/24 |
| `sensor.enedis_<prm>_autoconsommation` | % | Taux d'autoconsommation photovoltaïque |
| `sensor.enedis_<prm>_autoproduction` | % | Taux d'autoproduction / couverture solaire |
| `sensor.enedis_<prm>_prix_spot_day_ahead` | €/MWh | Cours horaire actuel du marché spot Day-Ahead |
| `sensor.enedis_<prm>_qualite` | - | Qualité métrologique du relevé (VALIDATED, etc.) |
| `sensor.enedis_<prm>_dernier_releve` | ISO8601 | Horodatage du dernier point de mesure |
| `sensor.enedis_<prm>_statut_synchronisation` | - | Statut de synchronisation avec l'API Enedis |

---

## 🌐 Accès à l'API REST et Ingress

Si `enable_web_api` est activé :
- Ouvrez le panneau latéral **Enedis Gateway** dans Home Assistant.
- Vous accédez directement à la documentation interactive **Swagger UI / OpenAPI**.
- Vous pouvez exporter vos données brutes en Parquet, DuckDB, JSON ou exécuter des audits personnalisés.
