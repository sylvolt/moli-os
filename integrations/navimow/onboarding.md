# Onboarding : Navimow

1. Déclarer `kind = "navimow"` (aucune option obligatoire ; `poll_secs`, 30 par défaut) ; `moli-os check-config`,
   redémarrer. La clé maître est nécessaire (la session est rangée dans le coffre).
2. Système → **Compte Navimow** → Ouvrir la page Navimow, se connecter avec le compte de l'appli.
3. Le navigateur arrive sur `https://my.home-assistant.io/redirect/oauth?code=…&state=…` (ou sur l'adresse de
   Home Assistant vers laquelle cette page renvoie) : copier l'adresse complète et la coller dans la carte.
4. Tester : la tondeuse apparaît (page Dehors), son état et sa batterie se mettent à jour ; un ordre (retour à
   la base) part et l'état suit en quelques secondes.
