# Vérification automatique du format

Objectif : qu'aucune modification (slicer, supports, import G-code, interface) puisse changer le format des
fichiers `.zcodex2` / `.zprojx` sans que ce soit détecté.

## Une commande

```powershell
.\verifier.ps1              # tout vérifier (≈ 1 min)
.\verifier.ps1 -Matrix 300  # avant une livraison : 300 combinaisons de réglages aléatoires
.\verifier.ps1 -Watch       # relance à chaque modification des sources
```
ou `cargo test --workspace --release`. Code de sortie ≠ 0 si quelque chose échoue.

## Les cinq niveaux

| Niveau | Quoi | Où |
|---|---|---|
| 1. **Garde à l'export** | Chaque fichier produit est vérifié *avant* d'être écrit ; un fichier invalide n'est jamais livré (message d'erreur précis à la place). Vaut pour l'interface, `ztool` et les scripts des slicers. | `zslice::export::package` → `zslice::verify` |
| 2. **Conteneur** | Octets : JSON d'en-tête, PNG valide, ZIP aux décalages **absolus** (ce que lit Z-Suite). | `verify::check_container` |
| 3. **Format Z-Suite** | Entrées présentes (dont `ConfigurationData`), 91 champs de `UserSettingsData` avec les bons types et des valeurs que Z-Suite sait retrouver dans ses listes, champs de l'en-tête. Ces règles viennent de la lecture du chargeur de Z-Suite. | `compat::check` |
| 4. **Flux de commandes** | Plateau 200×200, Z ≤ 180, températures 150–300 / 0–110, vitesses, **état de rétraction** (pas de double rétraction ni d'extrusion rétractée), marqueurs de couche consécutifs, progression croissante, extrusion plausible, en-tête cohérent avec le flux (nombre de commandes et de couches, emprise, matière). | `verify::check_stream` |
| 5. **Tests de non-régression** | voir ci-dessous | `zslice/tests/` |

## Tests de non-régression (`zslice/tests/`)
- **`golden.rs`** fige le *format* : entrées, champs et types de l'en-tête, début et fin du flux (types et valeurs fixes),
  types d'enregistrements utilisés (`golden/format.json`). Compare aussi avec un vrai fichier Z-Suite s'il est présent.
  Le chiffres clés d'un tranchage de référence (couches, commandes, durée, matière) peuvent bouger un peu quand
  l'algorithme s'améliore (tolérance 10 %), pas beaucoup.
  Après un changement **voulu** : relire le message d'échec, vérifier avec `ztool check`, puis
  `UPDATE_GOLDEN=1 cargo test -p zslice --test golden`.
- **`matrix.rs`** : combinaisons de réglages aléatoires (graine fixe, donc reproductibles) sur quatre formes (cube, T,
  arche, sphère sur pied) : chaque fichier doit passer la vérification complète, se relire à l'identique et survivre à
  un aller-retour par G-code. Des réglages invalides doivent être refusés, d'anciens fichiers de réglages doivent
  toujours se charger. `ZSLICE_MATRIX=N` règle le nombre de combinaisons.
- Tests unitaires : parseur de G-code, supports (arbre, volumes imposés/bloqués), conteneur `zcodex2` (relecture
  octet pour octet de vrais fichiers Z-Suite), chiffrement de `ConfigurationData`, etc.

## Ce que cela attrape (essais de sabotage faits)
- changer un octet du début du flux (ex. `M111(3)` → `M111(2)`) → `golden::format_is_unchanged` échoue en nommant le champ ;
- renommer un champ de l'en-tête → plusieurs tests échouent ;
- la matrice a trouvé un vrai défaut (marqueurs de couche non consécutifs et mauvaise épaisseur après une couche vide), corrigé ;
- la garde a trouvé qu'une progression pouvait précéder `Progress(0)` sur un G-code très court, corrigé.

## Ce que cela ne peut pas garantir
Le vérificateur reproduit **ce que le code de Z-Suite exige** et ce que les vrais fichiers contiennent ; il ne remplace pas
un essai réel : Z-Suite lui-même et l'imprimante n'ont pas pu être pilotés automatiquement. Après tout changement de
format, ouvre `zproj-studio/exemples/cube30.zcodex2` dans Z-Suite (sans imprimer).
