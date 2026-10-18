# Utiliser d'autres slicers avec la Zortrax M200 Plus

Le principe : le slicer produit un **G-code** classique, et `bin\ztool.exe` le convertit en **.zcodex2** (le format
que lit l'imprimante). Les températures, vitesses, rétractions, ventilateur, couches et types de trajectoires
(parois, remplissage, supports…) du G-code sont repris tels quels ; le début (mise en chauffe, repérage,
calibration) et la fin (arrêt) suivent la séquence de Z-Suite.

## Avant de commencer
- **Rien ici n'a été testé sur une imprimante, ni dans les slicers eux-mêmes.** Le convertisseur est testé sur
  des G-code écrits « à la manière » de PrusaSlicer et de Cura et sur des allers-retours (.zcodex2 → G-code →
  .zcodex2), pas sur de vrais fichiers de ces logiciels. Les profils (.ini / .json) sont des points de départ.
- Garde le plateau, les températures et la hauteur sous contrôle : la conversion **refuse** un G-code qui sort du
  plateau (200 × 200 mm), dépasse 180 mm de haut, ou demande une buse hors de 150–300 °C / un plateau hors de 0–110 °C.
- La première extrusion du G-code devient la première couche. Les lignes de purge du début de G-code sont
  converties comme le reste : évite-les (les profils ci-dessous n'en ont pas).
- Ignorés (avec un avertissement) : M600/M25 (pauses, changement de filament), M220, G28/G29 du slicer.
  Les arcs G2/G3 sont découpés en segments de 0,5 mm.

## Le plus simple : glisser-déposer
Glisse un `.gcode` sur `Convertir-en-zcodex2.bat` : le `.zcodex2` apparaît à côté. Autre matériau (si le G-code
n'a pas de températures) : `Convertir-en-zcodex2.bat fichier.gcode --material Z-ABS`.
ZProj Studio sait aussi lire un G-code (« Ouvrir un G-code… ») : aperçu couche par couche, puis conversion.

## En ligne de commande
```
bin\ztool.exe to-zcodex2 fichier.gcode [-o sortie.zcodex2] [--material Z-PLA] [--nozzle-temp 210] [--bed-temp 30]
bin\ztool.exe to-gcode   fichier.zcodex2 [-o sortie.gcode]      (Z-Suite -> G-code)
bin\ztool.exe info       fichier.zcodex2
bin\ztool.exe check      fichier.zcodex2      (vérifie que Z-Suite 3.7 devrait pouvoir l'ouvrir)
```

## PrusaSlicer / SuperSlicer / Slic3r
1. Fichier > Importer > *Importer la configuration groupée* : `PrusaSlicer\Zortrax_M200_Plus.ini`.
2. Réglages d'impression > Sorties > *Scripts de post-traitement* : `"C:\chemin\bin\ztool.exe" to-zcodex2`
   (le profil importé contient déjà cette ligne : remplace `CHEMIN_VERS_ZTOOL`).
3. Exporte le G-code : le `.zcodex2` est écrit à côté, avec le même nom (variable `SLIC3R_PP_OUTPUT_NAME`).

## OrcaSlicer (et BambuStudio)
1. Copie `OrcaSlicer\Zortrax M200 Plus 0.4 nozzle.json` dans
   `%APPDATA%\OrcaSlicer\user\default\machine\` (ou recrée l'imprimante à la main : plateau 200×200, hauteur 180,
   buse 0,4, G-code Marlin, rétraction 0,8 mm à 60 mm/s, saut en Z 0,4 mm).
2. Réglages du processus > Autres > *Scripts de post-traitement* : `"C:\chemin\bin\ztool.exe" to-zcodex2`.
3. Trancher et exporter le G-code : le `.zcodex2` est écrit à côté.

## Cura
1. Copie `Cura\definitions\*.def.json` dans `<dossier de configuration Cura>\definitions\` et
   `Cura\scripts\ZortraxZcodex2.py` dans `<dossier de configuration Cura>\scripts\`
   (Aide > Afficher le dossier de configuration). Redémarre Cura, ajoute l'imprimante « Zortrax M200 Plus ».
2. Extensions > Post-traitement > Modifier le G-code > Ajouter un script > *Zortrax .zcodex2* ;
   règle le chemin de `ztool.exe` et le dossier de sortie.
3. Après le découpage, le fichier est écrit dans le dossier de sortie (nom du travail + `.zcodex2`).
   Sans le script : enregistre le G-code puis glisse-le sur le `.bat`.

## Simplify3D
Processus > Scripts > *Additional terminal commands for post processing* :
`"C:\chemin\bin\ztool.exe" to-zcodex2 "[output_filepath]"`.
Dans « Fichier > G-code flavor », choisis Marlin, extrusion absolue (M82), plateau 200 × 200 × 180.

## IdeaMaker, KISSlicer, MatterControl, autres
Exporte le G-code (Marlin, extrusion absolue ou relative, unités en mm) puis glisse-le sur le `.bat`, ou ajoute la
commande `ztool.exe to-zcodex2 "<fichier>"` dans le champ « post-traitement » du logiciel s'il en a un.
Profil machine à recréer : plateau 200 × 200 mm, hauteur 180 mm, buse 0,4 mm, filament 1,75 mm, plateau chauffant.

## Réglages conseillés dans le slicer (valeurs mesurées sur des fichiers Z-Suite, Z-PLA 0,15 mm)
Première couche 0,26 mm · rétraction 0,8 mm à 60 mm/s · saut en Z 0,4 mm · buse 210 °C · plateau 30 °C ·
largeur de ligne 0,45 mm (parois), 0,6 mm (remplissage) · vitesses ~20 mm/s parois, ~70 mm/s remplissage.

## Exemple
`exemples\cube30_style_prusaslicer.gcode` : un cube de 30 mm (G-code écrit à partir d'un fichier Z-Suite-like)
pour essayer le `.bat`.
