# Suite du travail sur herd (lanceur llama-server)

Document de reprise. À lire avant toute extension du lanceur.

---

## Objectif du projet

herd est un **lanceur de LLM locaux en terminal**. Il transforme un
`models.ini` llama-server en quelque chose qui se parcourt et s'actionne, au
lieu d'un fichier qu'on lit avant de taper une longue ligne de commande.

Filiation : `llama-launch.js` résout la précédence ini et **imprime** un argv.
herd résout la même précédence, affiche l'argv en direct, puis lance et
supervise réellement le processus.

Du runner générique d'origine il ne reste que `sh`. `test` et `scan` répondaient
des chaînes fixes — `scan` n'a jamais regardé le réseau — et un `:help` qui les
annonce transforme un vestige inoffensif en promesse fausse. Le reste du
périmètre initial (méta-prompt multi-devices, système de plugins jamais
commencé, prompts de génération) et les compatibilités de renommage
(`$OPS_TUI_LLAMA_CONFIG`, `~/.config/ops-tui/session.json`) ont été supprimés
avec lui : l'historique est dans git, c'est là qu'appartient la trace de ce
qu'un projet a été.

## Les huit écrans

| | Écran | Rôle |
|---|---|---|
| `1` | Models | Table des presets du `models.ini` actif + aperçu argv en direct |
| `2` | Server | État du cycle de vie, endpoint, uptime, sortie récente |
| `3` | Router | Mode multi-modèles natif de llama-server + l'argv qu'il lancerait |
| `4` | Test | Appel de chat sur le modèle chargé : réponse, latence, débit |
| `5` | Stats | Compteurs de session, temps jusqu'au premier jeton, budget mémoire |
| `6` | Settings | Clés `[server]` / `[*]` / par modèle, éditables |
| `7` | Logs | Historique complet |
| `8` | Hub | Ce que llama.cpp a en cache : taille du modèle, disque du dépôt, preset qui l'utilise |

L'ordre du menu suit **la fréquence d'usage**, pas la parenté des écrans : les
sept premiers sont ceux que traverse une session, Hub est celui où l'on passe de
temps en temps pour voir ce que le cache retient.

Les chiffres sont **positionnels** : insérer ou déplacer un écran renumérote les
autres. Rien ne doit en coder un en dur — ni un test, ni une chaîne dans un
composant. L'ordre ne vit que dans `Screen::ALL`, donc déplacer un écran tient
en une ligne.

## Cycle de vie

```
OFF ──launch──> STARTING ──/health 200──> SERVING ──stop──> STOPPING ──> OFF
                    │                        │
                    └─── échec au spawn ─────┴── crash ──> ERROR ──> OFF
```

`STARTING → SERVING` est confirmé par un **polling de `GET /health`**, et non
plus par une heuristique sur les logs : llama.cpp reformule ses lignes de démarrage entre
versions, et un état affiché auquel on ne peut pas se fier est pire que pas
d'état du tout. `ERROR` existe pour qu'un OOM ou un GGUF manquant soit
distinguable d'un arrêt propre.

`stop_announced` n'émet **rien** quand rien ne tournait : afficher une
transition qui n'a pas eu lieu est un vrai bug d'IHM (test dédié).

## 🔴 Règle de verrouillage à ne jamais enfreindre

`Supervisor` garde l'enfant dans un `Arc<Mutex<Option<Child>>>`. **Aucun await
ne doit être fait en tenant ce verrou.**

Bug corrigé le 2026-08-10 : le watcher de sortie faisait `child.wait().await`
*sous* le guard, donc le verrou restait pris pendant toute la vie du processus.
Tout le reste qui en a besoin — le poller `/health`, `is_running`, `stop` — se
bloquait indéfiniment. Symptômes observés : l'IHM restait sur STARTING alors que
le serveur répondait déjà `200 {"status":"ok"}` sur `/health`, `:stop` ne
répondait plus, et la fermeture propre (`executor.shutdown()`) se bloquait aussi,
ce qui **laissait un llama-server orphelin sur le port 1234 en gardant la VRAM**.

Les deux règles qui en découlent :

- le watcher de sortie **poll `try_wait()`** par intervalles courts, il n'attend
  jamais `wait()` sous le verrou ;
- `stop()` **sort** l'enfant du slot d'abord, puis l'attend hors verrou.

Tests de non-régression : `a_live_child_never_holds_the_lock` (prouve que
`is_running` et `stop` répondent pendant qu'un enfant tourne) et
`a_healthy_process_transitions_to_serving` (STARTING → SERVING de bout en bout
contre un faux serveur HTTP, sans GPU).

## Génération de lancement

Chaque lancement porte un numéro de génération (`process.rs::Supervised`).
`stop()` comme un hot-swap vident puis re-remplissent le slot partagé : « y
a-t-il un enfant ? » ne suffit donc pas à dire à une tâche d'un lancement
précédent si l'enfant qu'elle voit est encore le sien. Sans ce garde-fou, un
watcher retiré adopte le processus suivant et le rapporte sous le nom du modèle
précédent. Garde : `is_current()`. Test :
`a_hot_swap_retires_the_previous_launch_watchers`.

## Marqueur de l'écran Models

Bug corrigé le 2026-08-10 : après un `stop`, le point restait devant le modèle
arrêté et le nom restait affiché. Deux causes conjuguées :

- `apply_status` ne vidait `server.model` que `if !was_live`, c'est-à-dire
  jamais dans le cas qui compte (SERVING → STOPPING → OFF) ;
- le marqueur se basait sur une simple égalité de nom, sans regarder l'état.

Désormais `apply_status` vide le modèle sur `Off` (et le **garde** sur `Error`,
pour pouvoir afficher « ERROR gemma4-12b »), et `lifecycle_glyph` dérive le
marqueur de l'état : `●` SERVING, `◐` STARTING/STOPPING, `✖` ERROR, rien sinon.
Tests : `a_stopped_model_loses_its_marker`, `each_live_state_has_its_own_marker`,
`stopping_then_launching_another_model_switches_cleanly`.

## Retours quand rien ne se passe

Deux pièges d'ergonomie corrigés en même temps :

- `App::run` ignorait silencieusement une commande quand une autre était en
  vol : on appuyait sur `s` puis `Entrée`, et la seconde touche disparaissait
  sans explication. Elle journalise maintenant `busy, ignored :<commande>`.
- `port_in_use_settled` re-teste le port trois fois à 250 ms d'intervalle avant
  de conclure : un serveur tué une seconde plus tôt peut encore accepter une
  connexion pendant que le noyau démonte la socket, ce qui déclenchait une
  modale « port occupé » contre un processus déjà mort.

## Formes de réponse de `/v1/models`

llama-server a livré deux formes : celle d'OpenAI (`{"data":[{"id":...}]}`) et
une forme façon Ollama (`{"models":[{"name":...,"model":...}]}`, ce que renvoie
le build 10330). `parse_model_list` accepte les deux. Avec seulement la première,
`:status` annonçait « no models currently loaded » face à un serveur qui servait
manifestement un modèle.

## Overrides de settings : jamais dans le `models.ini`

Les éditions de l'écran Settings s'appliquent au prochain lancement.
`models.ini` n'est **jamais** réécrit : ces fichiers sont maintenus à la main et
fortement commentés, et aucun round-tripper ini ne préserve fiablement
l'emplacement des commentaires.

Elles ne sont plus perdues à la fermeture pour autant : depuis `prefs.rs`, elles
sont conservées dans `~/.herd_config` (JSON trié, relisible et éditable à la
main), avec les favoris et les deux nombres du routeur. **La règle n'a pas
changé là où elle comptait** : elle portait sur l'*ini*, pas sur l'oubli.

Un override est exactement un override CLI, donc il s'insère dans la chaîne de
précédence existante sans nouvelle règle dans le moteur :

```
[server] → [*] → [model] → overrides de session → args CLI explicites
```

Deux fichiers, deux rôles à ne pas confondre :

- `~/.config/herd/session.json` : **où le programme en était** — le palier et le
  dernier preset lancé, rien d'autre.
- `~/.herd_config` : **ce que l'utilisateur a choisi** — favoris, overrides,
  nombres du routeur. L'écriture y signale ses échecs (contrairement au fichier
  de session) : perdre le palier est un désagrément, perdre un réglage posé
  exprès est une perte de travail.

La réservation mémoire (`+`/`-` sur Stats) reste volontairement hors des deux :
c'est une propriété de la machine du moment, pas un réglage de preset.

## Le dossier `data/`

Le dépôt embarque désormais un instantané des paliers de presets :

```
data/
├── 16gb/models.ini      13 presets (4B à 27B)
├── 32gb/models.ini       8 presets (12B à 35B)
├── scripts/llama-launch.js
├── scripts/test_call.sh
└── start-router.sh
```

Copie conforme de `~/models/`, aux deux fichiers près qui n'ont pas été repris :
`16gb/LLM hosting.md` et `huggingface`.

C'est de la **donnée de référence et de test, pas une source de configuration à
l'exécution** : la résolution lit toujours `~/models/`. Les tests parsent ces
fichiers directement via `CARGO_MANIFEST_DIR` :

- `shipped_16gb_tier_parses_with_every_preset` / `shipped_32gb_...` — la liste
  exacte des presets ;
- `every_shipped_preset_builds_an_argv` — chaque preset produit une ligne de
  commande lançable (source de modèle + alias) ;
- `shipped_tiers_share_a_port` — les deux paliers partagent bien le port, ce qui
  justifie la modale de conflit.

Si `data/` est resynchronisé depuis `~/models/`, ces tests sont ce qui signale un
preset dont la forme a changé. Les mettre à jour, pas les supprimer.

## Résolution de `models.ini`

1. `--config <path>` (aussi `-c`, `--config=`)
2. `$HERD_LLAMA_CONFIG`
3. le palier retenu de la session précédente, s'il existe encore
4. le palier de RAM détecté sous `~/models/<N>gb/`
5. l'ancien fichier plat `~/models/models.ini`

Les étapes 1 et 2 sont prises au pied de la lettre. L'étape 4 lit la RAM
installée (`sysctl hw.memsize` / `/proc/meminfo`, sans crate système) et retient
le palier le plus riche qui tient ; sinon le plus petit.

**Piège à ne pas rouvrir :** le chemin actif vit à deux endroits, `App` et
`Executor`. Changer de palier avec `t` renvoie `Action::ConfigPathChanged`, que
`main.rs` transmet à `Executor::set_config_path`. Si cette boucle est cassée,
les lancements résolvent les presets contre l'ancien palier et échouent en
« unknown model ». Test de non-régression :
`switching_tier_reports_the_new_config_path`.

## L'écran Test

Portage de `data/scripts/test_call.sh` : même `SYSTEM_PROMPT`, même message par
défaut (`Bonjour`), même requête non-streamée. Garder ces constantes alignées
sur le script — elles existent pour que les deux soient comparables.

La latence est mesurée localement, donc toujours affichée. `usage` et le bloc
non standard `timings` de llama.cpp sont lus de façon opportuniste : si le
serveur ne les fournit pas, la ligne se dégrade au lieu de disparaître.

La sonde voyage en `Action::RunChat { model, prompt }` et non en
`RunCommand(String)` : le prompt est du texte libre, il ne doit pas repasser par
un découpage de ligne de commande. Elle est volontairement **hors** du drapeau
`running` — une génération lente ne doit pas empêcher d'arrêter le serveur — d'où
son propre `chat_pending` et son propre `ChatGuard`, qui garantit exactement un
`ChatResult` même en cas de panique ou d'abandon.

Cible : le modèle chargé, sinon le preset sélectionné, pour rester utilisable
face à un serveur lancé hors d'herd.

## Dimensionnement mémoire (`services/llama/memory.rs`)

**Un preset déjà téléchargé est mesuré, plus estimé.** `LauncherState::sizing`
renvoie `Sizing::Measured` dès que le cache contient ce dépôt *et* cette
quantisation : le gguf de la révision pointée par `refs/main`, résolu à travers
les liens du snapshot, plus la même allocation d'exécution que l'estimation —
les deux lignes restent donc comparables et se jugent contre le même budget. La
table marque la différence : `7.3G` mesuré, `~18.3G` calculé.

Deux règles rendent la substitution sûre : la quantisation doit correspondre
(un Q4 en cache ne dit rien de la taille d'un Q8, et un chiffre précis et faux
est pire qu'une estimation annoncée comme telle), et il ne faut **pas** sommer
les blobs du dépôt — celui-ci conserve toutes les révisions déjà téléchargées,
ce qui annoncerait un modèle 12B au double de sa taille.

`estimate_gib` — le repli, pour ce qui n'est pas encore téléchargé — déduit le
nombre de paramètres et la quantisation du **nom du dépôt**, la seule
information de taille que porte un `models.ini`, puis ajoute la même allocation
forfaitaire. C'est une heuristique, assumée comme telle :

- le plus **grand** jeton `<n>B` gagne : un MoE `35B-A3B` compte pour 35B (tous
  les experts sont résidents), pas 3B ;
- les tags de quantisation ne doivent jamais passer pour un nombre de paramètres
  (`Q4_K_M` ne contient pas de `B`) — test dédié ;
- un nom illisible renvoie `None` → `Fit::Unknown` : **ne jamais signaler ce
  qu'on ne sait pas mesurer**, un avertissement rouge erroné est pire que pas
  d'avertissement. Idem si la RAM n'est pas lisible.

`Budget` = RAM installée moins `reserved_ratio` (25 % par défaut, l'ordre de
grandeur de ce que macOS garde hors du GPU). Session uniquement, contrairement
aux autres overrides. `+`/`-` sur l'écran Stats avancent par point de pourcentage entier —
la valeur est arrondie, sinon un `+= 0.05` répété dérive et n'atteint jamais
exactement les bornes.

Descendre sous le défaut active `is_risky()` : bandeau rouge permanent et
avertissement journalisé une fois. **herd ne modifie aucun réglage système** :
l'écran affiche `sudo sysctl iogpu.wired_limit_mb=…` à exécuter soi-même, il ne
le lance pas.

## Statistiques de session

`SessionStats` est remis à zéro à chaque `Starting` : les compteurs décrivent la
session de service courante. `average_rate` divise le total de jetons produits
par le temps total écoulé, plutôt que de moyenner les débits par requête — un
test fige cette distinction.

`started_at` est un `chrono::DateTime<Local>` conservé à côté de l'`Instant`
monotone : un `Instant` sait dire « il y a 12 min » mais jamais « démarré à
14:32 », ce que la page de stats demande. C'est la seule raison de la dépendance
`chrono`.

## Sélecteur de `models.ini` (`c`)

`c` ouvre une modale listant tous les `models.ini` de la machine, chacun avec son
nombre de presets et combien dépassent le budget courant. Les comptes sont
calculés en lisant les fichiers directement : l'intérêt du sélecteur est de juger
un fichier qui n'est **pas** celui chargé.

Choisir un autre fichier renvoie `Action::ConfigPathChanged` — même boucle que le
changement de palier, avec le même piège à ne pas rouvrir (voir plus haut).

## Conflits de port

Les paliers partagent `port = 1234`. Avant de lancer, l'`Executor` teste le port
et émet `UiEvent::PortInUse`, ce qui bascule l'App en `ConfirmLaunch`. La
confirmation re-dispatche en `launch!` (variante forcée, émise par la modale,
jamais tapée).

herd **ne tue jamais un processus qu'il n'a pas lancé** : il n'a aucun moyen
de savoir ce que c'est. Quand le port est tenu par son propre enfant supervisé,
`spawn` fait un hot-swap sans poser de question.

## État de validation

Validé sur la machine réelle (macOS / Apple Silicon) le 2026-08-10 :

```
cargo build                  # aucun warning
cargo test                   # 163/163 tests OK (+4 `live` ignorés)
cargo clippy --all-targets   # aucun warning
cargo fmt --check            # propre
```

Rendu vérifié contre le vrai `~/models/32gb/models.ini` : les 8 presets
s'affichent avec repo, ctx et mode spéculatif, et l'aperçu argv est correct.
Endpoints vérifiés contre un vrai llama-server (build 10330) : `/health`
renvoie bien `200 {"status":"ok"}` et `/v1/models` la forme « models ».

## Fiabilité du process llama-server (2026-08-11)

Sept correctifs, motivés par un MacBook Air M5 16 Go où le modèle frôle la
RAM disponible. Le détail des invariants est dans `CLAUDE.md` ; en résumé :

1. **La surveillance `/health` ne s'arrête plus au premier 200.** Un serveur
   qui se tait sans mourir était invisible pour le guetteur de sortie :
   l'IHM affichait SERVING indéfiniment. Après SERVING, sonde toutes les 5 s
   et signale `Phase::Unresponsive` après trois échecs consécutifs. Jamais
   escaladé en `Error` — le process est vivant et peut revenir.
2. **STARTING est détaillé.** `Phase::Binding` (rien n'écoute encore) vs
   `Phase::Loading` (503, poids en cours de chargement), avec deux budgets :
   90 s pour ouvrir le port, 600 s pour charger. Le temps écoulé est affiché
   dans la barre d'état — c'est ce qui distingue « ça charge » de « c'est
   planté ».
3. **`stop()` est borné.** SIGTERM (llama-server libère alors la mémoire GPU
   proprement) → 5 s → SIGKILL → 5 s → abandon. L'ancien `wait()` non borné
   était sur le chemin critique du hot-swap, de `:stop` et de la sortie de
   l'application : un kill lent gelait l'IHM.
4. **`:stop` contourne le verrou `running`.** C'est la commande dont on a
   besoin quand le reste est bloqué ; elle était refusée précisément dans ce
   cas.
5. **Les morts par signal sont diagnostiquées.** SIGKILL devient « killed by
   the system — most likely out of memory », complété par l'estimation et le
   budget quand le preset était déjà signalé trop gros.
6. **Un preset `Fit::TooLarge` demande confirmation avant lancement.** Le
   marqueur existait déjà mais ne bloquait rien.
7. **La boucle d'événements vide la file avant de dessiner**, et un seul
   client `reqwest` est partagé. Le chargement d'un modèle produit des
   centaines de lignes ; un rendu complet par ligne faisait ramer l'IHM au
   pire moment.

## Navigation (2026-08-11)

1. **Les touches page suivent la hauteur du terminal.** `App::page()` =
   hauteur − `chrome(écran)` − 1 ligne de recouvrement, au lieu d'un `PAGE`
   fixe à 10. La hauteur arrive par `UiEvent::Resize` ; `App::update` reste
   pure, elle est informée, elle n'interroge rien.
2. **Le sélecteur de config répond aux mêmes touches que les autres listes**
   (page, début, fin). Il réimplémentait `j`/`k` à la main et n'avait donc
   pas le reste.
3. **←/→ changent d'écran**, en doublon de Tab / Maj+Tab. Ces touches ne
   faisaient rien ; ↑/↓ ne peuvent pas jouer ce rôle, elles appartiennent
   aux listes.
4. **Indicateur de position `3/8`** sur Models et Settings, et **barre de
   défilement** sur la bordure droite de l'écran Logs (rien n'est dessiné
   quand tout le tampon tient à l'écran).
5. **Entrée bascule les booléens** sur Settings (`true/false`, `on/off`,
   `yes/no`), au lieu d'ouvrir l'éditeur. `1`/`0` sont exclus : impossibles
   à distinguer d'un réglage numérique. La casse et la famille d'écriture
   sont conservées (`ON` → `OFF`, jamais `false`). Case à cocher
   `[x]`/`[ ]` devant les valeurs concernées.
6. **Chronométrage sur l'écran Test** : heure d'envoi, latence, et le
   découpage serveur de llama.cpp (`prompt eval` / `generation` /
   `overhead`). Compteur qui défile pendant l'attente.

## Disponibilité locale des modèles (2026-08-11)

1. **`llama-server --cache-list` fait autorité**, pas une inspection du
   disque : llama.cpp refuse à juste titre de lister un dépôt dont les
   blobs contiennent un `.downloadInProgress` inachevé (cas réel de
   `gemma-4-31B` sur cette machine), ce qu'aucun `find` n'aurait vu.
   `Availability::Unknown` tant que la réponse n'est pas arrivée : annoncer
   à tort « à télécharger » est la seule erreur coûteuse ici.
2. **Colonne LOCAL** sur l'écran Models. Sur le palier 16gb, 10 presets sur
   13 ne sont pas présents localement.
3. **Entrée sur un preset absent demande confirmation**, puis télécharge
   *et* lance. **`d`** télécharge sans lancer.
4. **Téléchargement délégué au CLI `hf`** : il possède le format du cache
   (blobs, liens de snapshot, refs), c'est la partie qu'il ne faut pas
   rater. Fichiers nommés explicitement, jamais de glob.
5. **La progression est mesurée, pas analysée.** Dans un tube, `hf` 1.27
   n'affiche qu'un compteur de *fichiers* (`Fetching 3 files: 33%`) : un
   fichier de 6,7 Go resterait à 0 % du début à la fin. On somme donc les
   blobs terminés et les `*.incomplete` du cache, face au total donné par
   l'API tree.
6. **Deux pièges révélés par les vraies données**, tous deux couverts par
   des tests : `unsloth/gemma-4-*-GGUF` contient un *répertoire* nommé
   `MTP` (sélectionné à tort comme artefact de 0 octet), et expose deux
   `mmproj` (BF16 et F16) dont un seul est nécessaire.

## Disponibilité, arrêt et coût à vide (2026-08-11, suite)

1. **`--cache-list` fait autorité, et le téléchargement est vérifié.** Un code
   de sortie 0 de `hf` ne prouve pas que le modèle est utilisable : c'est
   llama.cpp qui en décide. On re-interroge donc le cache après coup, au lieu
   d'annoncer « downloaded » pendant que la ligne reste « not local ».
2. **Télécharger n'est pas échouer à écouter.** llama-server télécharge ses
   propres poids quand un lancement ne les trouve pas ; 16 Gio dépassent
   largement `BIND_BUDGET`, et la sonde déclarait donc mort un téléchargement
   parfaitement sain au bout de 90 s. `Phase::Downloading` nomme cet état, les
   budgets partent du dernier octet reçu, et seuls les octets postérieurs au
   lancement comptent (un téléchargement tué laisse son partiel derrière lui).
3. **Course à l'arrêt.** La sonde vérifiait « ce lancement est-il toujours le
   mien » *avant* de sonder, puis attendait jusqu'à 3 s. Un `stop` tombant dans
   cette fenêtre avait déjà annoncé OFF ; la sonde écrivait par-dessus et
   l'application restait en ERROR. Elle re-vérifie désormais avant d'émettre.
   Et `s` efface une erreur périmée : `is_live()` étant faux, rien ne
   l'effaçait auparavant.
4. **Coût à vide mesuré.** 10,5 Mio RSS — négligeable face aux 8–17 Gio du
   serveur, donc le runtime est laissé tel quel. En revanche le tick de 250 ms
   redessinait 4 fois par seconde un écran inchangé : **80 ms → 30 ms de CPU
   par 30 s**. Un tick ne redessine plus que si une horloge est à l'écran.
   Essayé puis **annulé** : réduire les features tokio (aucun gain mesurable).

## Cache local, tailles mesurées, TTFT et `:help` (2026-08-12)

Quatre points de la TODO, dont trois s'appuient sur le même fait nouveau : le
cache de llama.cpp est **mesurable**, il n'a plus à être deviné.

- **Écran Hub** (`2`). Models dit ce que ce palier sait lancer, Hub dit ce que la
  machine possède, et les deux listes diffèrent dans les deux sens. Colonnes
  `SIZE` (les poids que llama.cpp chargerait) et `DISK` (tout ce que le dépôt
  occupe, révisions périmées comprises) : sur cette machine, 6,3G de modèle dans
  13,1G de répertoire. `*` sur `DISK` = deux quantisations partagent le
  répertoire, dit plutôt que divisé — le cache ne tient aucune comptabilité par
  quantisation. Les modèles qu'aucun preset du palier ne nomme sont en cyan
  (pas en rouge : ce n'est pas une erreur, ils peuvent appartenir à un autre
  palier). `y` copie une strophe `models.ini` prête à coller. **Aucune touche de
  suppression, volontairement** : libérer 17 Gio ne se propose pas à une touche
  de `j`, et herd ne touche pas à ce qu'il n'a pas posé.
- **Tailles mesurées** (voir la section mémoire ci-dessus).
- **TTFT** sur Stats, à côté du débit : un modèle qui pagine ses poids génère à
  un rythme honorable et n'affiche pourtant rien pendant quatre secondes.
  Dérivé (`latency - predicted_ms`), la sonde étant non-streaming par choix ;
  sans `timings` la ligne affiche `-` et sa raison, jamais un zéro. La ligne
  porte **trois chiffres et le premier est à froid** — voir la section datée
  plus bas.
- **Ascenseur** sur les listes Models et Hub quand elles débordent, jamais
  sinon. Sa position reproduit le décalage que `List` va choisir, sinon la
  glissière contredirait les lignes qu'elle décrit.
- **`:help`** ouvre la liste des commandes en surimpression. La table
  (`commands.rs`) est désormais l'unique endroit où une commande est écrite, et
  elle est **vérifiée contre les répartiteurs** par deux tests : la liste
  précédente (`scripts.rs`) avait dérivé — ni `reload`, ni `cache`, et un
  « écran 3 » devenu faux à l'insertion du Router.

## Profil `[mono-focus]` et un bug d'argv (2026-08-12)

Un jeu de flags nommé, activable **par preset**, pour ce que l'ini ne sait pas
dire autrement : un seul client qui boucle sur le même prompt de base, et qui
veut garder son cache KV plutôt que de le partager.

**C'est un nom de section réservé**, comme `server` et `*`, et c'est tout le
mécanisme : toute autre section du fichier est un preset, donc sans réservation
le profil apparaîtrait dans l'écran Models comme un modèle lançable sans
`hf-repo` — compté dans le palier et proposé à `Enter`. `ini::is_reserved` est
le seul endroit où cette liste vit.

Précédence : `[server] → [*] → [model] → mono-focus → overrides → CLI`. **Après**
le preset, puisqu'on l'active justement pour forcer un comportement que la
section du modèle contredit ; **avant** les overrides, pour qu'une seule de ses
clés puisse être reprise depuis l'écran Settings sans toucher au fichier.

L'interrupteur est un *choix* : il vit dans `~/.herd_config` à côté des favoris,
par nom de preset, désactivé par défaut. Les clés du profil ne sont listées
**que lorsqu'il est actif** — des lignes qui ont l'air éditables sans être en
vigueur valent moins que pas de lignes du tout — et l'en-tête porte l'état, une
section absente et une section désactivée étant sinon indiscernables.

**Le bug trouvé en chemin.** `argv_preview` appliquait les overrides de session
et `Executor::spawn_launch` ne les appliquait pas : l'écran Models affichait un
`--ctx-size 65536` que le processus lancé ne voyait jamais. Silencieusement,
depuis que l'écran Settings existe. `LaunchSettings::argv` est désormais le seul
endroit où l'argv d'un lancement est assemblé, et l'Executor reçoit ces réglages
avant chaque commande — même forme que `set_config_path`, même raison.

## L'aperçu argv défile et s'ajuste à son cadre (2026-08-12)

L'aperçu était tronqué dans les deux sens, et c'est exactement la faute que
`Columns::for_width` et `screen_hint_within` existent pour empêcher ailleurs.

**En largeur.** `wrap_argv` coupait à 40 colonnes en dur et ne mesurait que le
*drapeau* pour décider de couper : `--hf-repo` tenait, et la référence de dépôt
de 42 caractères qui suit débordait, coupée par le terminal. Il s'ajuste
désormais à la largeur réelle du cadre et mesure **l'option entière, drapeau et
valeur ensemble** — une option est une unité, jamais scindée. Une option plus
longue que le cadre est repliée ici, pas laissée au terminal : le nombre de
lignes rendu est ainsi celui qui est compté.

**En hauteur.** Six lignes suffisent à un preset ordinaire ; un preset avec
`[mono-focus]` actif, ou un terminal en 80 colonnes, non. `J`/`K` font défiler,
une barre apparaît sur la bordure droite, et **les deux seulement quand quelque
chose est caché** — annoncer une touche qui ne ferait rien est un mensonge de
plus.

Le défilement est borné **dans `App::update`**, pas au dessin : `render` reste
une fonction pure de `App`, et l'autre solution est un compteur qui grimpe
pendant que la vue ne bouge plus. `App` doit donc connaître la géométrie du
cadre — `preview_pane` la compte à la main comme `chrome`, d'où la largeur
ajoutée à `UiEvent::Resize`. Le rendu reborne contre le cadre réellement obtenu,
et un redimensionnement reborne aussi : un terminal plus large replie le même
argv en moins de lignes.

## Les listes qui débordent : Settings (2026-08-12)

Hub avait déjà sa barre (`list_scrollbar`, arrivée avec l'écran) ; il lui
manquait un test, elle en a un. Settings n'avait ni l'une ni l'autre moitié.

**La barre doit se compter en lignes, pas en éléments.** Un en-tête de section
est une ligne vide plus un titre : deux lignes pour un seul `ListItem`. Le
`list_scrollbar` existant suppose une ligne par élément et aurait décalé le
curseur d'une ligne par en-tête au-dessus du curseur — trois sur cet écran.
`tall_list_scrollbar` prend les hauteurs ; `tall_viewport_top` en est la partie
pure, testée directement, et reproduit la règle de ratatui plutôt que de la
deviner.

**En largeur aussi.** Une valeur plus longue que le cadre était coupée par le
terminal — `unsloth/gemma-4-12B-it-qat-GGUF:UD-…` en 80 colonnes se lit comme
une référence qui s'arrête là. Les lignes sont désormais coupées avec une
marque, et un test parcourt quatre largeurs.

Au passage : `truncate` était écrit **trois** fois et les copies avaient dérivé
(l'une sans garde `width == 0`, donc une ellipse d'un caractère pour une colonne
de largeur nulle — un débordement, pas une coupe). Il n'y en a plus qu'un, dans
`components`. Celui de Hub coupait par le début, ce qui est le bon choix pour
une référence de dépôt : il s'appelle `elide_start`.

## TTFT : trois chiffres, le premier à froid (2026-08-12)

```
TTFT        4.20s  (Time to First Token) · last 0.35s · avg 1.63s
```

`first_token` ne retient que **la première sonde après le chargement** — la
seule qui mesure un modèle dont les poids ne sont pas encore résidents. `last`
et `avg` couvrent toutes les sondes qui ont rapporté un `timings` et décrivent
le modèle **chaud** : l'autre moitié de la question, ce que coûte une requête
une fois le modèle en place.

Les deux sont tenus séparés plutôt que fondus en un seul nombre : une moyenne
sur l'ensemble dérive vers la valeur à chaud à mesure qu'on sonde, et ne décrit
alors ni l'une ni l'autre. Tout est remis à zéro à chaque `Starting`.

Si la *première* sonde ne rapporte pas de `timings`, le chiffre de tête reste
`-` et les chiffres à chaud s'affichent quand même : promouvoir la seconde
donnerait un nombre chaud portant une étiquette froide.

## « Aucun serveur ne tourne » (2026-08-12)

Trois fonctions exigent un llama-server en service — `:status`, `:ping <model>`
(et `p` sur l'écran Server), et la sonde de l'écran Test — et toutes trois
échouaient dans le vocabulaire de la tuyauterie :

```
:status -> http://127.0.0.1:1234 unreachable: request failed: error sending
           request for url (http://127.0.0.1:1234/v1/models)
```

Deux problèmes sur une ligne, et aucune des deux moitiés ne dit que la solution
est de lancer quelque chose. `api::unreachable` classe désormais l'échec : une
connexion refusée devient « nothing is listening on <base> — no llama-server is
running (start one with :launch <model>, or :router) » ; un timeout reste un
timeout (quelque chose *a* répondu, c'est un autre problème) ; le reste est
aplati par sa chaîne de causes au lieu d'être tronqué au message de surface de
reqwest. L'URL est toujours nommée : un serveur sur le mauvais port ressemble
exactement à une absence de serveur.

Le refus est détecté de deux façons (`api::refused`) : `is_connect()`, et le
`io::ErrorKind` en dessous. La première est le test documenté, mais cette
classification a déjà bougé d'une version de reqwest à l'autre ; l'erreur io, non.

**Pas de refus a priori**, même quand herd sait que son propre `ServerState` est
`Off` : sonder un serveur lancé hors de herd est un usage assumé, et un serveur
mort il y a une seconde aussi. On tente, puis on explique.

`p` sur l'écran Server était le cas muet : sans rien qui tourne il n'y a pas de
nom de modèle à envoyer, et la touche renvoyait `None` sans un mot — impossible
à distinguer d'une touche non liée.

## `:about` (2026-08-12)

Troisième surimpression locale à côté de `?` (les touches) et `:help` (les
commandes), pour la troisième question d'un utilisateur bloqué : « qu'est-ce que
je fais tourner ? ». C'est `--version` à l'écran, plus les faits qui décident du
comportement sur *cette* machine — `models.ini` chargé, palier, RAM détectée et
budget qui en découle, répertoire de cache. Tout y est déjà ailleurs (la barre
latérale a la version, le titre de Models le chemin, l'écran Stats le budget) :
justement, répondre à cette question ne devrait pas être une visite de quatre
écrans, et c'est ce qu'un rapport de bug doit contenir.

La ligne « arbre modifié » n'apparaît que lorsqu'elle est vraie. Les chemins sont
tronqués **par la gauche** : c'est la fin qui identifie un chemin, et une valeur
coupée par la bordure se lirait comme une valeur qui s'arrête là.

## Suppression d'un modèle en cache (2026-08-12)

`D` sur le Hub, **seule touche destructive du programme**. Elle était
volontairement absente ; ce qui la rend acceptable est le prompt, pas un
changement d'avis sur le risque. Quatre garde-fous :

- **Majuscule**, parce que `d` en minuscule sur l'écran Models *télécharge* : le
  même doigt signifiant « récupère ça » sur un écran et « détruis ça » sur le
  suivant, c'est exactement comme ça qu'un accident arrive. Les majuscules sont
  déjà les variantes fortes (`Q`, `X`).
- **Le prompt chiffre avant de demander** : la taille, et toute autre
  quantisation du même répertoire qui partirait avec (`also_removes`). Un prompt
  qui emporte un second modèle en silence n'a pas posé la question à laquelle
  l'utilisateur a répondu.
- **Seul un `y` minuscule confirme**, contrairement aux prompts de lancement qui
  acceptent aussi `Y` : une touche majuscule partie toute seule ne doit pas être
  ce qui détruit un téléchargement.
- **Deux refus secs**, pas des avertissements : le dépôt qu'un serveur vivant
  est en train de servir, et celui qu'un téléchargement est en train d'écrire.
  Aucun des deux n'a de « oui quand même » sensé.

Le chemin est calculé par `repo_dir`, jamais pris dans une saisie, et
`delete_repo` vérifie encore qu'il est directement sous le répertoire du cache,
qu'il porte le préfixe `models--` et qu'il existe. Un test lui présente `""`,
`..` et des traversées, et vérifie qu'un répertoire voisin survit.

Ensuite l'Executor **relit le cache** au lieu de croire sa propre suppression :
une suppression à moitié réussie apparaît comme une ligne toujours listée,
plutôt que comme un écran qui contredit le disque en silence.

## Version à chaque commit (2026-08-12)

`hooks/pre-commit`, installé par `make hooks` (qui pointe `core.hooksPath` sur
le dossier versionné `hooks/`). Un commit qui **fixe déjà** une version est
laissé tel quel, ce qui garde `make release` utile ; `HERD_NO_BUMP=1` saute un
coup ; le hook **refuse** plutôt que d'indexer `Cargo.toml` en bloc si le
fichier porte des modifications non indexées.

Un hook et non `build.rs` : un script de build qui réécrit `Cargo.toml` salit
l'arbre à chaque compilation, invalide sa propre empreinte et reboucle — et il
compterait des *builds*, pas des changements.

## Limites connues à améliorer

1. **Pas de redémarrage automatique en cas de crash.** `ServerState::Error` est
   affiché, rien n'est retenté. Un serveur devenu muet
   (`Phase::Unresponsive`) est signalé mais jamais relancé non plus : la
   décision reste à l'utilisateur.
2. **Tests d'intégration partiels.** `api.rs` a désormais des tests `live`
   marqués `#[ignore]` (health / list_models / port_in_use) à lancer avec
   `cargo test -- --ignored --test-threads=1` quand un serveur tourne sur
   :1234, et `process.rs` valide STARTING → SERVING contre un faux serveur
   HTTP. Manque encore un test bout en bout contre un vrai `llama-server`
   (spawn + chargement de modèle + kill).
3. **~~Le prompt de port occupé ne couvre pas le mode routeur.~~ Corrigé.**
   `:router` pose désormais la même question que `launch` quand le port est
   tenu par un processus que herd n'a pas lancé (variante forcée cachée
   `router!`, même esprit que `launch!`). Au passage : le hot-swap annonce
   STOPPING avant d'arrêter le serveur précédent — l'arrêt silencieux d'un
   gros modèle en train de paginer ressemblait à un blocage du routeur au
   démarrage — et un arrêt qui dépasse sa grâce (`Stopped::Abandoned`)
   refuse le spawn en nommant la cause, au lieu de laisser l'erreur de bind
   brute de llama-server surgir une seconde plus tard.
4. **L'estimation mémoire reste une heuristique pour ce qui n'est pas
   téléchargé.** Un preset présent en cache est maintenant mesuré sur son
   fichier ; les autres sont toujours calculés à partir du nom, et le cache KV
   effectif (fonction de `ctx-size` et du nombre de couches) n'est modélisé dans
   aucun des deux cas. Pistes : l'API tree de HuggingFace pour les absents — un
   appel réseau par ligne, donc à ne faire que si l'estimation se révèle fausse
   sur un preset qui compte — et un modèle de KV à partir de `ctx-size`.
5. **Doublons entre paliers non signalés dans l'IHM.** `gemma4-12b`,
   `qwen3-vl-8b-instruct` et `qwen-3-14b-instruct` existent dans 16gb et 32gb ;
   rien ne l'indique à l'écran (un test le vérifie côté données seulement).
6. **Logs sans recherche.** L'écran Logs défile désormais (`App::log_scroll`,
   500 lignes conservées) mais n'offre ni recherche ni filtre.
7. **`:status` ne montre pas le modèle réellement chargé en mémoire par le
   routeur** au-delà de ce que renvoie `/v1/models`.
8. **La vision est sous-déclarée.** `no-mmproj` n'est pas lu comme la preuve
   d'un projecteur (il est posé défensivement, y compris sur des modèles sans
   vision) : une capacité n'est retenue que si le *nom* la porte. `gemma-4`
   embarque donc un mmproj sans le dire et passe pour texte seul. Le correctif
   honnête est de chercher un `mmproj` dans le listing du dépôt, pas de deviner
   plus fort.
9. **La suppression retire le dépôt entier, pas une quantisation.** `D` sur le
   Hub efface `models--<dépôt>` en bloc : le cache ne tient aucune comptabilité
   par quantisation, et aller extraire des blobs d'un répertoire partagé à la
   main est le meilleur moyen de le corrompre. Le prompt nomme donc ce qui part
   avec. Une suppression par quantisation demanderait de reconstruire les liens
   du snapshot, ce que ni `hf` ni llama.cpp n'exposent.

## Méthode

Avant de conclure quoi que ce soit :

```
cargo build && cargo test && cargo clippy --all-targets && cargo fmt
```

La convention du projet est de partir au vert et de laisser au vert.
