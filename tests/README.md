# Test Suite – localhost HTTP server

## Prérequis

- Python 3.6+
- `siege` pour les stress tests : `sudo apt install siege`
- Le serveur doit être démarré **avant** de lancer les tests.

## Lancer les tests

### Étape 1 : démarrer le serveur (dans WSL)
```bash
cd /mnt/c/Users/axelw/localhost
~/.cargo/bin/cargo run
```

### Étape 2 : lancer le script de tests (dans un 2e terminal WSL)
```bash
cd /mnt/c/Users/axelw/localhost
python3 tests/run_tests.py
```

Ou sur un port/host différent :
```bash
python3 tests/run_tests.py 127.0.0.1 8080
```

## Catégories de tests

| # | Catégorie | Critère d'audit |
|---|-----------|-----------------|
| 1 | Disponibilité du serveur | Le serveur ne doit jamais crasher |
| 2 | Méthodes HTTP (GET, POST, DELETE, FOOBAR) | Méthodes obligatoires |
| 3 | Pages d'erreur (400, 404, 405) | Erreurs personnalisées |
| 4 | Headers HTTP | Content-Type, Content-Length, HTTP/1.1 |
| 5 | Upload de fichier (multipart/form-data) | Upload de fichiers |
| 6 | Limite taille body (>2 Mo → 413) | client_max_body_size |
| 7 | Redirections (301/302) | Directive `return` |
| 8 | Listing de répertoire / index | autoindex on/off |
| 9 | Cookies & Sessions | Set-Cookie / Cookie |
| 10 | Chunked request | Transfer-Encoding: chunked |
| 11 | Timeout client lent | Toutes les requêtes ont un timeout |
| 12 | Ports multiples | listen multiple ports |
| 13 | Virtual hosts (server_name) | Header Host distingué |
| 14 | CGI (.py, .php) | cgi_extension |
| 15 | Stabilité (50 requêtes rapides) | Disponibilité |
| 16 | Siege stress test (10s) | ≥ 99.5% availability |
| 17 | Mémoire (/proc RSS) | Pas de fuite mémoire |

## Notes sur les SKIP

Les tests marqués `[SKIP]` ne sont pas des échecs – ils indiquent que la fonctionnalité n'est pas encore configurée.  
Pour les activer, adapte `server.conf` (ex: ajouter un `upload_store`, un `return`, etc.).
