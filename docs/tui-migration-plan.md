# Plan de migration vers une TUI dynamique

## Contexte

Maestro expose actuellement son activité via `tracing` (stdout plaintext ou JSON) et un endpoint Prometheus. L'objectif est d'ajouter un mode d'affichage TUI moderne (inspiré de lazygit, btop, helix) pour remplacer/compléter les logs défilants lorsque le binaire tourne dans un terminal interactif.

## État des lieux

L'architecture actuelle est déjà bien positionnée pour une migration TUI :

- Un `EventBus` existe dans `crates/core/src/events/` avec des canaux `broadcast` (événements) et `watch` (état courant).
- L'indexer émet déjà des événements structurés sans connaître ses consommateurs : `IndexerEvent`, `BackfillEvent`, `HandlerEvent`, `ChainEvent`.
- Deux consommateurs indépendants existent déjà : `logger` et `metrics_bridge`, tous deux spawnés comme tâches tokio dans `bin/maestro/src/main.rs`.
- Les `watch` channels exposent en continu `CursorState { head, tail }` et `ChainState { connected, finalized_head, spec_version }`.

**Conséquence :** la TUI sera un troisième consommateur au même niveau que le logger, sans modification du cœur de l'indexer ni des handlers.

## Ce qui manque

1. **Dépendances TUI** — ni `ratatui` ni `crossterm` ne sont dans le workspace `Cargo.toml`.
2. **Module `tui/`** dans `bin/maestro/src/` pour contenir l'état, le rendu et l'input.
3. **Routage des logs** — `tracing_subscriber::fmt()` écrit sur stdout (`bin/maestro/src/main.rs:321`), ce qui entre en conflit direct avec ratatui en mode raw. Il faut une couche `tracing` qui pousse vers un buffer partagé affiché dans un panneau.
4. **Flag d'activation** — `--tui` (ou auto-détection `is_terminal()`) avec fallback vers le mode actuel pour CI, `--json-logs`, non-TTY.
5. **Gestion du prompt `--purge`** — actuellement `io::stdin().read_line()` (`bin/maestro/src/main.rs:391`) ; incompatible avec crossterm raw mode. Le mode purge restera en affichage classique (pas de TUI).

## Architecture proposée

```
bin/maestro/src/tui/
├── mod.rs          # pub fn run(event_bus, shutdown) -> JoinHandle
├── app.rs          # AppState : agrège tous les events en état observable
├── render.rs       # draw(frame, &AppState) : layout ratatui + widgets
├── input.rs        # gestion clavier (q/quit, tab/switch, scroll)
└── log_layer.rs    # tracing Layer qui pousse dans un ring buffer partagé
```

### Layout suggéré

| Zone | Contenu |
|------|---------|
| Header | Statut RPC (connected/reconnecting), chain, spec_version, uptime |
| Gauge backfill | `[tail ━━━━━░░░░ head]` avec %, ETA, blocs/sec |
| Stats live | Dernier bloc indexé, latence moyenne, extrinsics/events du bloc courant |
| Handlers | Tableau par pallet : events processed, persisted, errors |
| Logs | Panneau scrollable avec filtre par niveau |
| Footer | Raccourcis (`q` quit, `Tab` switch panel, `/` filter) |

### Intégration avec l'existant

- La TUI est spawnée dans `bin/maestro/src/main.rs` comme une 4ᵉ tâche tokio, au même endroit que `logger` et `metrics_bridge`.
- Elle reçoit des handles clonés sur les mêmes canaux `broadcast` et `watch` que les autres consommateurs.
- Shutdown via le `watch::channel(false)` déjà utilisé par les autres tâches ; un `Drop` guard restaure le terminal même en cas de panic.

## Plan d'implémentation (incrémental)

### Phase 1 — Squelette

- Ajouter `ratatui` et `crossterm` à `bin/maestro/Cargo.toml`.
- Créer le module `tui/` avec un `run()` qui :
  - Entre en mode raw crossterm et alt-screen.
  - Boucle à ~30 Hz via `tokio::select!` (tick + shutdown).
  - Affiche un layout statique (panneaux vides).
- Ajouter le flag `--tui` dans `cli.rs`.
- Spawner la tâche depuis `main.rs` derrière le flag.
- Implémenter le `Drop` guard pour restaurer le terminal.

### Phase 2 — État réactif

- Définir `AppState` agrégeant :
  - `CursorState`, `ChainState` (depuis les `watch` channels).
  - Compteurs handlers (`HashMap<pallet, HandlerStats>`).
  - Dernier bloc indexé, latence moyenne glissante.
  - Plan de backfill (ranges, progress).
- S'abonner aux 4 canaux `broadcast` de l'`EventBus`.
- Mettre à jour l'état sur chaque événement reçu dans la boucle `select!`.
- Câbler le rendu sur l'état réel.

### Phase 3 — Logs ✅

- ~~Écrire une `tracing_subscriber::Layer` custom qui pousse dans un `Arc<Mutex<VecDeque<LogLine>>>`.~~ Fait dans `tui/log_layer.rs` (custom plutôt que `tui-logger` : pas de dépendance supplémentaire, ~160 lignes contrôlées).
- ~~N'activer cette layer qu'en mode TUI ; sinon fallback sur `fmt()`.~~ `init_tracing` retourne `Option<LogBuffer>`, seul le chemin TUI installe `TuiLogLayer`.
- ~~Exposer le buffer partagé dans `AppState` et rendre un panneau scrollable.~~ Panneau rendu en bas de la zone principale, slice basé sur hauteur + offset.
- ~~Ajouter filtrage par niveau (`info`/`warn`/`error`) via input clavier.~~ `f` cycle `all → info+ → warn+ → error`, `PgUp/PgDn`/`↑↓` scroll, `End`/`g` suit le tail.

### Phase 4 — Polish ✅

- ~~Couleurs thématiques cohérentes (palette unique).~~ Fait : module `tui::theme` expose `PRIMARY`, `SUCCESS`, `WARN`, `ERROR`, `DEBUG`, `MUTED`, `PRIMARY_FG`. Plus aucun `Color::…` littéral en dehors de ce module.
- ~~Scroll, filtres interactifs, ETA backfill calculé sur la vitesse glissante.~~ `AppState` maintient un historique `(Instant, persisted)` borné (64 samples / 30 s). `backfill_rate()` et `backfill_eta()` dérivent la vitesse glissante ; le libellé de la jauge affiche `b/s` et `ETA`.
- ~~Gestion propre du `SIGINT` (Ctrl+C) : forwarder au shutdown signal global.~~ `main` fait un `tokio::select!` entre `shutdown_signal()` et un `wait_for_shutdown(&mut main_shutdown_rx)`, de sorte qu'un quit TUI (qui pousse `shutdown_tx.send(true)`) réveille aussi `main` — indispensable en raw mode où Ctrl+C ne déclenche plus le handler OS.
- ~~Tests : rendu du layout sur `AppState` fixtures (ratatui supporte les tests de buffer).~~ `tui::tests` utilise `ratatui::backend::TestBackend` + `flatten()` pour vérifier en-tête, jauge, panneaux stats/handlers/logs et footer. `app::tests` couvre la logique de rate/ETA (injection de `Instant`).

### Phase 5 — Câblage producteurs ✅

**Contexte.** L'état des lieux du document affirmait « L'indexer émet déjà des événements structurés ». En pratique, seul `PalletHandlerExt` appelle `EventBus::emit_handler`. `IndexerService`, `BackfillRunner` et `SubstrateClient` n'ont aucune référence au bus. Résultat observé en Phase 4 : TUI lancée → `disconnected`, `mode –`, `head 0 · tail 0`, `Last block –`, jauge backfill absente. Tous les panneaux sauf *Logs* restent inertes parce que les watches `cursor`/`chain` ne sont jamais publiées et qu'aucun `IndexerEvent`/`BackfillEvent`/`ChainEvent` n'est diffusé.

**Objectif.** Brancher les producteurs existants sur le bus sans toucher à l'architecture : tout le plombing (`emit_*`, `publish_cursor`, `publish_chain`) est déjà disponible — il suffit d'appeler aux bons endroits.

#### Travaux — `crates/core/src/services/indexer.rs`

- Ajouter un champ `event_bus: EventBus` à `IndexerService`, propagé via `IndexerService::new`.
- Au tout début de `run()` :
  - publier `ChainState { connected: true, finalized_head, spec_version }` sur le watch (`bus.publish_chain(...)`) en réutilisant `block_source.finalized_head()` + `runtime_version()` déjà appelés par `main.rs` — OU les relire ici.
  - émettre `IndexerEvent::Started { mode, start_block }`.
- À la sortie de `run()` : émettre `IndexerEvent::Stopped { reason }` selon le chemin (shutdown vs erreur fatale). Soit via un wrapper `async fn run_inner(...)` appelé depuis `run()`, soit avec un drop-guard-like pattern.
- Juste avant d'entrer dans la boucle live après backfill : émettre `IndexerEvent::LiveModeEntered { from_block }`.
- Dans `index_single_block` (appelée par live ET backfill) après `persist_block_atomic` :
  - émettre `IndexerEvent::BlockIndexed { number, hash, extrinsics, events, duration_ms }` (en utilisant le `ProcessingTimer` déjà présent pour la durée).
  - calculer la nouvelle `CursorState { head, tail }` à partir du cursor post-persist, émettre `CursorAdvanced` + `publish_cursor`.
- Dans `run_live_loop` :
  - sur `Ok(stream)` → émettre `ChainEvent::RpcConnected { url: chain_id/ws_url }` (ajouter un champ `ws_url: String` à `IndexerConfig` si nécessaire, ou retirer l'argument et passer l'URL autrement ; simple : stocker l'URL dans `IndexerConfig`).
  - sur erreur de subscription ou rupture de stream → émettre `ChainEvent::RpcDisconnected { reason }`, publier `ChainState { connected: false, .. }`.
  - sur backoff retry → émettre `ChainEvent::RpcReconnecting { attempt }`.
  - sur reconnexion réussie → re-publier `ChainState { connected: true, .. }`.

#### Travaux — `crates/core/src/services/backfill.rs`

- Ajouter `event_bus: EventBus` à `BackfillRunner`, propagé via `BackfillRunner::new`.
- Émettre `BackfillEvent::Planned { from, to, total }` une fois le plan calculé (dans `run_range` ou en amont — idéalement une seule fois sur l'ensemble agrégé, donc à faire dans `IndexerService::run` juste avant la boucle `for range in plan.ranges`).
- Dans `run_range` :
  - après chaque `persist_block_atomic` réussi → `BackfillEvent::BlockPersisted { number }`.
  - sur retry fetch → `BackfillEvent::FetchRetried { number, attempt, error }`.
  - sur abort (budget épuisé, erreur fatale) → `BackfillEvent::Aborted { reason }` avant de remonter l'erreur.

#### Travaux — `bin/maestro/src/main.rs`

- Cloner `event_bus` dans `IndexerService::new(...)` (argument supplémentaire) et — s'il est construit séparément — `BackfillRunner::new(...)`. En pratique `BackfillRunner` est créé à l'intérieur de `IndexerService::run`, donc on ne passe qu'une fois le bus à `IndexerService::new` et le service le repasse au runner.

#### Tests

- Mettre à jour les tests unitaires existants de `indexer.rs` et `backfill.rs` qui appellent `::new(...)` — passer un `EventBus::new(DEFAULT_BUS_CAPACITY)` dummy. Mécanique, pas de changement de logique.
- Ajouter un test d'intégration léger dans `crates/core/tests/` qui :
  - construit un `IndexerService` avec des stubs `BlockSource` + `Repositories`,
  - branche des `subscribe_*` sur le bus,
  - fait tourner un bloc synthétique,
  - assert qu'on reçoit `Started` + `BlockIndexed` + `CursorAdvanced` + une publication `ChainState { connected: true, .. }`.

#### Vérification manuelle

- Relancer `cargo run --bin maestro -- --tui` contre un nœud local et vérifier :
  - header bascule `disconnected` → `connected`, `spec v0` → `spec vN`, `finalized 0` → numéro courant,
  - `mode` affiche `backfill` puis `live`,
  - `Stats` → `Last block`, `Avg latency` se remplissent,
  - `Handlers` → la ligne `Balances` s'anime dès le premier event traité,
  - jauge backfill visible et animée avec `b/s` + `ETA` dès qu'il y a un range historique.

#### Coût estimé

- ~150-200 lignes nettes réparties entre `indexer.rs` (~80), `backfill.rs` (~30), `main.rs` (~4), tests (~30-50).
- 0 refonte architecturale — purement additif.
- Risque principal : oublier un chemin d'émission (ex : `ChainState` au startup, ou `Stopped` sur un early return), d'où le test d'intégration recommandé.

#### Résultat

- `IndexerService` porte un `event_bus` + un `Arc<Mutex<Option<CursorState>>>` mirror. `run()` wrappe `run_inner()` pour garantir l'émission de `Stopped` + une transition `ChainState { connected: false, .. }` sur tout chemin de sortie.
- `run_inner` publie au startup `ChainState { connected: true, finalized_head, spec_version }` (relit `finalized_head()` + `runtime_version()`), seed le cursor mirror depuis le cursor persistant, puis émet `Started { mode, start_block }`. En `live_only`, émet aussi `LiveModeEntered` avec `from_block` dérivé du cursor.
- Avant la boucle `for range in plan.ranges`, `run_inner` émet un `BackfillEvent::Planned { from, to, total }` agrégé sur tous les ranges.
- `index_single_block` capture un `Instant` local, émet `BlockIndexed { number, hash, extrinsics, events, duration_ms }`, met à jour le cursor mirror (`head = max`, `tail = min`), publie `CursorState` sur le watch, et émet `CursorAdvanced`.
- `run_live_loop` maintient un compteur `reconnect_attempt`, émet `RpcConnected { url: config.ws_url }` et republie `ChainState { connected: true, .. }` sur chaque subscription réussie, émet `RpcDisconnected { reason }` + republie `connected: false` sur erreur de subscribe ou rupture de stream, et émet `RpcReconnecting { attempt }` dans le select! de backoff.
- `BackfillRunner::new` prend l'`EventBus`. `run_range` émet `BlockFetched` / `BlockPersisted` par bloc, `RangeCompleted` en fin de range, et `Aborted { reason }` avant de remonter `BackfillAborted`. `fetch_with_retry` accepte un `Option<&EventBus>` et émet `FetchRetried { number, attempt, error }` à chaque retry.
- `IndexerConfig` gagne un champ `ws_url: String` (alimenté depuis `Cli::to_indexer_config`). `main.rs` passe `event_bus.clone()` à `IndexerService::new`.
- Tests : `crates/core/tests/indexer_events.rs` construit un `IndexerService` avec des stubs `BlockSource`/`Repositories` en mémoire, fait tourner un bloc synthétique et vérifie les traces `ChainState`/`CursorState` + les `IndexerEvent` (`Started`, `BlockIndexed`, `CursorAdvanced`). `backfill_integration.rs` est mis à jour pour le nouveau `BackfillRunner::new`. Tests `fetch_with_retry` adaptés à la signature `Option<&EventBus>`.

## Tradeoffs à trancher

### `ratatui` pur vs `ratatui` + `tui-logger`

- **Pur** : contrôle total, pas de dépendance supplémentaire, mais ~100 lignes pour ring buffer + filtrage + scroll.
- **`tui-logger`** : s'intègre directement comme `tracing-subscriber` layer, widget prêt à l'emploi, une dépendance de plus.

**Recommandation :** `tui-logger` pour la Phase 3 — gain de temps significatif, maintenance réduite.

### Activation automatique vs flag explicite

- **Auto via `is_terminal()`** : meilleure UX en local, mais risque de casse si un utilisateur pipe la sortie ou utilise un terminal inhabituel.
- **Flag `--tui` explicite** : prévisible, mais friction supplémentaire.

**Recommandation :** flag explicite `--tui` en Phase 1, envisager l'auto-détection plus tard si demandé.

## Coût estimé

- **~600-900 lignes** de code neuf, concentrées dans `bin/maestro/src/tui/`.
- **0 modification** du core, des handlers, du storage ou du GraphQL.
- **2 dépendances** ajoutées (`ratatui`, `crossterm`) + éventuellement `tui-logger`.

## Points de vigilance

- **Restauration du terminal sur panic** : critique. Utiliser un `Drop` guard dans `run()` qui appelle `disable_raw_mode()` et `LeaveAlternateScreen` même en cas d'unwinding.
- **Conflit stdout** : aucun `println!` / `eprintln!` ne doit s'exécuter pendant que la TUI est active. Grep sur `println!`/`eprintln!` à faire avant la Phase 1.
- **Mode `--json-logs`** : doit désactiver la TUI automatiquement (la sortie JSON est destinée au pipe/fichier).
- **Mode `--migrate-only`, `--purge`, `--export-schema`** : doivent rester en affichage classique, la TUI ne s'active que pour le mode indexing long-running.
