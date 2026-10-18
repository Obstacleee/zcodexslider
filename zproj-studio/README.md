# ZProj Studio

Importe des STL, les affiche en 3D, exporte un projet Z-Suite `.zprojx` **et** un fichier d'impression `.zcodex2`
(découpeur intégré, crate `../zslice`).

- Lancer : `ZProjStudio.exe` (ou `cargo run --release`). Glisser-déposer d'un `.stl` / `.zprojx` possible.
- Trois onglets : **① Modèle** (import, position, taille, plateau), **② Réglages** (mode *Simple* avec curseurs : matériau,
  qualité, solidité, supports, adhérence, ventilateur ; mode *Avancé* avec tous les réglages), **③ Exporter**.
- Ce qui dépasse du plateau est **en rouge** dans la vue 3D (seule la partie hors plateau) ; l'export est refusé tant que la
  pièce dépasse. Le radeau, le bord et les supports sont réduits automatiquement pour rester sur le plateau.
- Les **supports, le radeau et le bord** sont calculés en arrière-plan et affichés en 3D (cases « Supports » / « Radeau / bord »).
- Vue 3D : glisser = tourner, clic droit = déplacer, molette = zoom, double-clic = recadrer.
- Onglet « Couches » : trajectoires de chaque couche (parois, remplissage, peaux, ponts, supports, radeau).
- Section 5 : tous les réglages (matériau, températures, couches, parois, remplissage, vitesses, débit, rétraction,
  radeau/bord, supports, ponts), enregistrables en JSON.
- Supports : colonnes ou **arbre** (branches qui s'écartent du modèle et fusionnent), motif (lignes/grille/concentrique),
  expansion, surface minimale, plancher plein, volumes **imposés** (vert) / **bloqués** (rouge) dessinés dans la vue 3D.
- « Ouvrir un G-code… » : lit le G-code d'un autre slicer (aperçu des couches) puis le convertit en `.zcodex2`.
  Les profils et scripts pour PrusaSlicer, OrcaSlicer, Cura, Simplify3D sont dans `../integrations/`.
- Sans interface : `cargo run --release --example stl2zcodex2 -- in.stl out.zcodex2 [support] [noraft]` (dans `../zslice`).

## À savoir avant d'imprimer
- Les `.zcodex2` générés **n'ont pas été validés sur une imprimante ni ouverts dans Z-Suite**. Ouvre d'abord
  `exemples/cube30.zcodex2` dans Z-Suite (sans imprimer), puis surveille la première couche.
- `ConfigurationData` est générée automatiquement (chiffrée avec la clé publique de Zortrax, comme Z-Suite) : Z-Suite
  refuse d'ouvrir un fichier qui n'en a pas. Seul Zortrax peut la déchiffrer. Option : recopier celle d'un fichier Z-Suite.
- Vérifie un fichier avant de l'ouvrir dans Z-Suite : `ztool.exe check fichier.zcodex2` (entrées, champs et valeurs attendus).
- Seule la M200 Plus + Z-PLA a des valeurs vérifiées sur des fichiers Z-Suite. Les autres matériaux ont des températures
  typiques (marquées dans l'interface).
- Pas (encore) gérés : couture réglable au point près, éviter les surplombs, vase (spirale), parois fines,
  stabilisation de température, tour de refroidissement, anneau anti-warping, supports hybrides, pauses,
  double extrusion (M300 Dual), imprimantes résine.

## Vérification automatique
`verifier.bat` (ou `.erifier.ps1`) à la racine lance tous les tests et vérifie les fichiers produits : voir `VERIFICATION.md`.
Chaque fichier `.zcodex2` / `.zprojx` est aussi vérifié par l'application **avant** d'être écrit ; un fichier invalide n'est jamais livré.
