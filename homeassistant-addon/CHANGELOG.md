# Journal des modifications (Changelog)

## [0.1.0] - 2026-09-30
### Ajouté
- Première version officielle du module complémentaire Home Assistant pour `enedis-rs`.
- Prise en charge des architectures multi-CPU : `aarch64` (Raspberry Pi 4/5), `amd64`, `armv7`.
- Enregistrement automatique de 12 entités Home Assistant par protocole MQTT Discovery.
- Intégration directe au Tableau de bord Énergie (Energy Dashboard).
- Remontée des analyses de talon de veille, solaire en autoconsommation et cours du marché Spot.
- Intégration Ingress pour un accès direct au serveur REST et à Swagger UI sans exposer de port externe.
