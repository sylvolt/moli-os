# Victron Energy (GX)

- Le broker MQTT local d'un GX (Cerbo GX, Venus OS sur Raspberry Pi…), « MQTT on LAN » activé
  dans les réglages du GX. Venus OS publie chaque valeur D-Bus sous
  `N/<portail>/<service>/<instance>/<chemin>` (`{"value": …}`) tant qu'un client envoie un
  keepalive (`R/<portail>/keepalive`, toutes les 30 s ici) ; écritures sur `W/…`.
- Un appareil Moli par service vu : batterie (`battery`), régulateur solaire (`solarcharger`),
  onduleur-chargeur (`vebus`), borne (`evcharger`), compteur réseau (`grid`), onduleur PV
  (`pvinverter`), système (`system`), réglages ESS (`settings`). Points relus : `TABLE` de
  `crates/moli-victron`.
- **Écritures relues, une par une**, toutes en commande de machine (sémantique `control` :
  un agent, l'assistant ou un automatisme attend un humain) : mode de l'onduleur ; borne
  (charge autorisée, mode, courant 6 à 32 A) ; charge minimale de l'ESS (10 à 100 %, par 5).
  Jamais une protection (tension de charge, limites de la batterie).
- Réutilisé : les chemins, unités et codes d'état de victron_mqtt (Apache-2.0, voir `NOTICE`) ;
  aucun code copié.
- `fixtures/topics.txt` : relevé sur un vrai GX (Venus OS 3.8x, batterie CAN 48 V,
  SmartSolar 150/35, MultiPlus-II, borne EVCS), novembre 2026 ; portail remplacé.
- Vérifié en lecture sur ce GX. Écritures : d'après les chemins documentés, pas encore
  essayées sur une vraie installation.
