# Guide de Contribution

Merci de votre intérêt pour ce projet ! Ce guide vous aide à comprendre comment contribuer correctement.

## 🚀 Avant de Commencer

### Prérequis
- **Rust 1.70+** via [rustup](https://rustup.rs/)
- **Git** configuré
- **Linux/WSL** (le projet utilise epoll)
- **Python 3.8+** (pour les tests)

### Configuration Locale

```bash
# Cloner le dépôt
git clone <repo-url>
cd localhost

# Compiler le projet
cargo build

# Lancer les tests
python3 tests/run_tests.py 127.0.0.1 8080
```

## 📋 Structure du Projet

```
src/
├── main.rs           # Point d'entrée
├── config/           # Parsing de configuration
├── http/             # Protocole HTTP (request, response, routing)
├── server/           # Logique réseau (reactor epoll, client state machine)
└── ...

tests/
├── run_tests.py      # Suite de test automatisée
└── configs/          # Configurations de test

www/
├── index.html        # Page d'accueil
├── cgi-bin/          # Scripts CGI
├── uploads/          # Répertoire des uploads
└── errors/           # Pages d'erreur personnalisées
```

## 🔄 Processus de Développement

### 1. Créer une branche
```bash
git checkout -b feature/description-courte
# ou pour une correction
git checkout -b fix/description-courte
```

### 2. Faire vos changements
- ✅ Respectez le style de code existant
- ✅ Testez localement avec `cargo build` et les tests Python
- ✅ Écrivez des messages de commit clairs (voir section Commits)

### 3. Tester avant de commit
```bash
# Compiler sans warnings
cargo build

# Lancer tous les tests
python3 tests/run_tests.py 127.0.0.1 8080

# Format du code (optionnel mais recommandé)
cargo fmt --check
```

### 4. Commit et Push
```bash
git add .
git commit -m "feat: description courte et claire"
git push origin feature/description-courte
```

### 5. Ouvrir une Pull Request
- Décrivez clairement les changements
- Référencez les issues associées (#123)
- Assurez-vous que les tests passent

## 📝 Messages de Commit

Nous suivons le standard **Conventional Commits** :

```
<type>(<scope>): <description>

<body optionnel>

<footer optionnel>
```

### Types
- **feat**: Nouvelle fonctionnalité
- **fix**: Correction de bug
- **docs**: Modifications de documentation
- **refactor**: Refactorisation du code
- **perf**: Améliorations de performance
- **test**: Ajout/modification de tests
- **chore**: Maintenance, dépendances, build

### Exemples

```
feat(router): add support for PUT method

This allows clients to update resources via PUT requests.
Implements RFC 7231 PUT semantics.

Closes #42
```

```
fix(client): prevent body size overflow

Check body size limit during read phase, not after parse.
Returns 413 Payload Too Large when limit exceeded.

Fixes #123
```

```
docs: update README with installation instructions
```

```
chore: upgrade libc to 0.2.200
```

## 🧪 Tests

Tous les changements doivent passer les tests existants. Pour ajouter des tests :

```bash
# Modifier tests/run_tests.py
# ou créer une config de test dans tests/configs/
```

**Résultats attendus** : 24 PASS, 0 FAIL, 1 SKIP (siege)

## ✋ Règles de Contribution

- ✅ Ne modifiez que ce que vous comprenez
- ✅ Testez complètement avant de soumettre
- ✅ Respectez le code existant
- ✅ Documentez les nouvelles fonctionnalités
- ✅ Signalez les limitations connues

- ❌ Ne commitez pas les fichiers de build (`target/`)
- ❌ Ne commitez pas les logs ou fichiers temporaires
- ❌ Ne commitez pas les secrets ou tokens
- ❌ Ne modifiez pas `.gitignore` sans raison valide

## 🐛 Signaler un Bug

Créez une issue GitHub avec :
1. **Description claire** du problème
2. **Étapes de reproduction** (commandes exactes)
3. **Comportement attendu vs actuel**
4. **Environnement** (OS, version Rust, etc.)

Exemple :
```
Titre: DELETE sur répertoire retourne 405 au lieu de 204

Étapes:
1. mkdir -p www/test_dir
2. curl -X DELETE http://localhost:8080/test_dir
3. Reçoit 405 Method Not Allowed

Attendu: 204 No Content ou 400 (répertoire non vide)
Actuel: 405 (comme si DELETE était désactivé)

Env: Ubuntu 22.04, Rust 1.93.1
```

## 📖 Documentation

- **README.md** : Vue d'ensemble et guide d'utilisation
- **CONTRIBUTING.md** : Ce fichier
- **Code comments** : Documentez les parties complexes
- **FIXES_SUMMARY.md** : Historique des optimisations

## 🚀 Avant de Tag une Release

1. ✅ Tous les tests passent (`24 PASS`)
2. ✅ README à jour
3. ✅ Version bumped dans `Cargo.toml`
4. ✅ CHANGELOG mis à jour (si applicable)

Format de version : `MAJOR.MINOR.PATCH` (semver)

```bash
# Exemple: de 0.1.0 à 0.2.0
cargo add . --vers=0.2.0 # met à jour Cargo.toml
git tag v0.2.0
git push origin v0.2.0
```

## 📞 Besoin d'Aide ?

- 📖 Consultez le README.md
- 🔍 Cherchez dans les issues
- 💬 Créez une discussion

---

**Merci de contribuer !** 🎉
