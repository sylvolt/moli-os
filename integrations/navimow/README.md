# Segway Navimow (tondeuses robots)

- Par le **cloud Navimow**, l'API que Segway ouvre à son intégration Home Assistant officielle
  ([NavimowHA](https://github.com/segwaynavimow/NavimowHA)) : la liste des tondeuses, leur état et leur
  batterie, et cinq ordres (tondre, arrêter, pause, reprendre, retour à la base). Dépendant d'Internet.
- **Connexion** depuis Système → Compte Navimow : la page de connexion de Navimow (mot de passe chez
  Navimow, jamais vu par Moli), puis l'adresse où elle aboutit (`https://my.home-assistant.io/redirect/oauth?code=…`)
  collée dans Moli. Moli se présente comme l'intégration Home Assistant (son identifiant client, le secret
  que Segway publie avec elle, son adresse de retour) : c'est la seule porte que Navimow ouvre, et elle peut
  changer sans prévenir.
- **Jetons** : le jeton d'accès dure environ une heure ; Moli le renouvelle avant son expiration avec le
  jeton de renouvellement quand Navimow en donne un, et range la session dans le coffre. Sans jeton de
  renouvellement, ou s'il est refusé (signalé par d'autres sur l'intégration officielle), il faut se
  reconnecter.
- **API** : le schéma Smart Home de Google dans l'enveloppe de Navimow (`{code: 1, data: {payload}}`) :
  `GET /openapi/smarthome/authList` (les tondeuses), `POST /openapi/smarthome/getVehicleStatus` (l'état :
  `vehicleState` comme `isRunning`, `isDocked`, `isPaused`, `isDocking`, `isLifted`… ; la batterie dans
  `capacityRemaining`), `POST /openapi/smarthome/sendCommands` (`StartStop`, `PauseUnpause`, `Dock` ;
  `alreadyInState` vaut un succès).
- **Lu toutes les 30 s** (`poll_secs`), et 4 s après un ordre. Le MQTT temps réel de Navimow n'est pas
  encore utilisé (l'intégration officielle retombe elle aussi sur cette relève quand il se tait).
- Pas de position ni de zones : l'API publique ne les donne pas pour les modèles sans LiDAR (i105).
- `fixtures/` : réponses **reconstituées** d'après le SDK officiel et le schéma de Google, pas encore
  comparées à un vrai compte.
