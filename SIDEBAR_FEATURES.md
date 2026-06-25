# Fonctionnalités ajoutées : sidebars & diff

Récapitulatif des fonctionnalités ajoutées au fork de Helix durant cette session.

## Vue d'ensemble

Trois fonctionnalités principales, inspirées de NvChad, VSCode et Zed :

1. **Explorateur de fichiers** — sidebar gauche pour naviguer/éditer l'arborescence (style nvim-tree).
2. **Panneau de changements git** — sidebar gauche listant les fichiers modifiés, groupés et en arbre (style Zed).
3. ~~**Vue diff côte-à-côte**~~ — retirée pour l'instant (`Entrée` ouvre simplement le fichier).

Les deux sidebars de gauche sont **mutuellement exclusives** (une seule affichée à la fois) et ont la **même largeur** (30 colonnes).

---

## 1. Explorateur de fichiers (`helix-term/src/ui/explorer.rs`)

Composant `ExplorerSidebar` rendu dans une bande à gauche. L'éditeur recalcule
ses splits dans la zone restante, donc tout se réarrange automatiquement.

- **Arbre aplati** : la liste ne contient que les nœuds visibles ; déplier insère
  les enfants, replier les retire. Lecture du système de fichiers paresseuse.
- **Statut git par fichier** (couleurs), propagé aux **dossiers parents** (façon VSCode) :
  un dossier contenant un changement est coloré. Priorité d'agrégation : Modified > Deleted > Added.
- **`reload` préserve** l'état déplié et la sélection.
- **`reveal`** : à l'ouverture depuis un fichier, déplie les dossiers parents et sélectionne ce fichier.

### Opérations fichiers (via la command line + autocomplétion de chemin)

| Touche | Action |
|---|---|
| `a` | Créer un fichier (ou dossier si le chemin finit par `/`) — crée aussi les dossiers parents |
| `r` | Renommer / déplacer |
| `d` | Supprimer (récursif pour les dossiers) |

Ces opérations utilisent `editor.create_path` / `move_path` / `delete_path`
(avec notifications LSP), puis rafraîchissent l'arbre via `job::dispatch_blocking`.

### Navigation

| Touche | Action |
|---|---|
| `j` / `k`, ↑ / ↓ | naviguer |
| `l` / `Entrée` / → | déplier un dossier / ouvrir un fichier |
| `h` / ← | replier / remonter au parent |
| `W` | tout replier |
| `R` | recharger |
| `q` / `Échap` | rendre le focus à l'éditeur |

### Raccourcis d'ouverture

- **`Ctrl-e`** → toggle (montre/cache, garde l'état).
- **`Space e`** → focus (ouvre + focus sur le fichier courant, dossiers parents dépliés). Remplace l'ancien picker `file_explorer`.

---

## 2. Panneau de changements git (`helix-term/src/ui/changes.rs`)

Composant `ChangesSidebar`, ouvert via **`Space g`** (remplace `changed_file_picker`).
`Space g` se comporte comme `Space e` : ouvre+focus si fermée, focus si ouverte mais
qu'on est sur un buffer, ferme si déjà focus.

**Navigation inter-sidebars / command line** : quand une sidebar a le focus, la plupart
des touches lui sont routées, mais le *leader* (`space`) et la command line (`:` / `;`)
retombent sur le keymap normal. On peut donc faire `Space g` depuis l'explorer pour
basculer sur les changes (et `Space e` inversement), ou ouvrir `:`/`;` sans quitter la
sidebar (routage dans `EditorView::handle_event`).

- Interroge `editor.diff_providers.changed_files()` et **regroupe** en :
  **Added** (untracked), **Modified** (modifié/conflit/renommé), **Deleted**. Compte affiché : `Modified (3)`.
- **Arborescence compressée façon Zed** : les chaînes de dossiers à enfant unique
  sont fusionnées en une seule ligne (`src/routes/api.export`).
- Noms colorés par statut (mêmes couleurs que l'explorer) + sigil `+` / `~` / `-`.
- **`Entrée` sur un fichier l'ouvre simplement dans l'éditeur** (`Action::Replace`), comme un
  picker classique — pas de diff, pas de split.

### Navigation

| Touche | Action |
|---|---|
| `j` / `k`, ↑ / ↓ | naviguer |
| `l` / `Entrée` / → | déplier groupe/dossier, ou **ouvrir le fichier** |
| `h` / ← | replier |
| `R` | rafraîchir |
| `q` / `Échap` | rendre le focus |

---

## 3. Vue diff côte-à-côte — retirée pour l'instant

La vue diff (panneau / buffer côte-à-côte) a été **retirée** : `Entrée` ouvre directement
le fichier (voir §2). Le code est récupérable dans l'historique git si on souhaite y revenir.

---

## Couleurs (statut git)

Surchargables via le thème, avec ces valeurs par défaut :

| Statut | Clé de thème | Couleur |
|---|---|---|
| Ajouté | `version_control.added` | `#27A657` (vert) |
| Modifié | `version_control.modified` | `#D3B020` (jaune) |
| Supprimé | `version_control.deleted` | `#E06C76` (rouge) |

---

## Fichiers touchés

**Nouveaux**
- `helix-term/src/ui/explorer.rs`
- `helix-term/src/ui/changes.rs`

**Modifiés**
- `helix-term/src/ui/mod.rs` — déclaration/export des modules
- `helix-term/src/ui/editor.rs` — sidebars `explorer`/`changes`, routage clavier avec **passthrough** (leader/command line depuis une sidebar), `toggle_changes` (focus si ouverte)
- `helix-term/src/ui/changes.rs` — `focus_panel`/`unfocus`, `Entrée` → ouvre le fichier (`Action::Replace`)
- `helix-term/src/commands.rs` — commandes `toggle_file_explorer`, `focus_file_explorer`, `toggle_changes_sidebar`
- `helix-term/src/keymap/default.rs` — bindings `Ctrl-e`, `Space e`, `Space g`
- `helix-vcs/src/lib.rs` — méthode synchrone `DiffProviderRegistry::changed_files()`

> La vue diff (et ses dépendances : `imara-diff`, `Editor::open_diff`/`DiffSession`,
> `Document::name_override`, `goto_diff_file`) a été retirée — récupérable via git.
