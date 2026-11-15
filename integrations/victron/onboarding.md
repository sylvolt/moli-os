# Onboarding : Victron Energy (GX)

1. Sur le GX : Réglages → Services → **MQTT** (« MQTT on LAN », en clair). Demander l'adresse
   IP du GX (de préférence fixe).
2. Déclarer `kind = "victron"`, `host = "<ip>"` (`portal_id` facultatif : sinon le premier GX
   annoncé) ; `moli-os check-config`, redémarrer.
3. Tester : les appareils apparaissent (batterie, MPPT, onduleur, borne…) avec leurs valeurs.
4. Énergie : `battery_…/charged_energy` et `discharged_energy` en compteurs `charge` /
   `discharge`, `battery_…/soc` et `power` dans `[energy.battery]` ; `solarcharger_…/yield_today`
   en compteur `solar` (remis à zéro chaque jour : `reset = "auto"`).
5. Écritures : toutes demandent une approbation humaine depuis un agent ou un automatisme.
   Les essayer d'abord sur une cible sans risque (le courant de la borne sans voiture branchée)
   et remettre la valeur d'avant.
