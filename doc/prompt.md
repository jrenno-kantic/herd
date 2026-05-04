# Méta-prompt — Ops TUI : console unifiée multi-devices

## 🎯 Vision

Construire une TUI (Terminal User Interface) hybride combinant les paradigmes de `htop`, `raycast` et `tmux` pour piloter un environnement multi-devices depuis une console unique.

## 📦 Périmètre fonctionnel (MVP)

### Gestion des devices
- Découverte et listing : Mac, Flipper Zero, iPhone
- État temps réel de chaque device

### Commandes rapides
- Envoi de scripts vers Flipper Zero
- Toggle du hotspot iPhone
- Lancement d'applications (cross-device)

### Observabilité
- Logs temps réel multi-sources
- Streaming non-bloquant

### UX & Navigation
- Navigation principale via **flèches haut / bas**
- Raccourcis clavier vim-like en complément (`hjkl`, `:`, `/`)
- Palette de commandes type Raycast (mode `:` pour saisie)
- **Commande `:help`** affichant la liste exhaustive des commandes disponibles avec leur description
- Help contextuel selon le panneau actif

### Exemples d'interactions
```
> flipper send nfc badge.office
> iphone hotspot on
> switch launch zelda
> :help
```

## 🎨 Design TUI premium

Références d'inspiration :
- **`lazygit`** — densité d'information, panneaux multiples, navigation fluide
- **`k9s`** — navigation contextuelle, raccourcis intuitifs, theming soigné

## 🏗️ Stack technique

Contexte temporel : **mai 2026** — utiliser les dernières versions stables.

- **Langage** : Rust (édition la plus récente)
- **TUI** : `ratatui`
- **Async runtime** : `tokio`
- **Architecture** : modulaire, non-bloquante, event-driven

## 📚 Méthodologie & ressources

1. **Spécifications** : structurer le projet selon [spec-kit](https://github.com/github/spec-kit) → générer `./ops-tui-spec-kit`
2. **Skills Karpathy** : intégrer les patterns de [andrej-karpathy-skills](https://github.com/forrestchang/andrej-karpathy-skills)

## 📖 Documentation

Le `README.md` doit impérativement contenir :
- **Présentation** du projet et de sa valeur
- **Installation** (prérequis, build, run)
- **Use cases concrets** :
  - Pousser un badge NFC depuis le Mac vers le Flipper en une commande
  - Activer le hotspot iPhone à distance pour partager la connexion
  - Lancer une app sur la Switch (ou autre device) sans changer de contexte
  - Centraliser les logs de plusieurs devices dans une seule fenêtre
- **How-to / Quickstart** :
  - Premier lancement et découverte des devices
  - Navigation (flèches haut/bas, raccourcis vim-like)
  - Utilisation de la palette de commandes (`:`)
  - Accès à l'aide intégrée (`:help`)
- **Tableau des raccourcis clavier**
- **Architecture** (vue d'ensemble des modules)

---

## 🚀 Phase de build

Générer une application Rust TUI basée sur le spec-kit présent dans `./ops-tui-spec-kit`.

**Contraintes techniques :**
- `ratatui` + `tokio` obligatoires
- Respecter les interactions définies dans `tui/interactions.md`
- UI non-bloquante avec mises à jour asynchrones
- Architecture modulaire (séparation `devices / commands / ui / events`)
- Navigation principale aux flèches ↑ / ↓
- Commande `:help` listant toutes les commandes disponibles avec description

**Livrables initiaux :**
- Structure du projet
- `main.rs` fonctionnel
- `README.md` complet (présentation, installation, use cases, how-to, raccourcis, architecture)