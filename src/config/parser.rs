use super::{Config, HttpMethod, RouteConfig, ServerConfig};
// use std::collections::HashMap;

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    ParseError(String),
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

pub fn parse_file(path: &str) -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    parse_string(&content)
}

pub fn parse_string(content: &str) -> Result<Config, ConfigError> {
    let mut tokens = tokenize(content);
    let mut config = Config::new();
    
    while let Some(token) = tokens.next() {
        if token == "server" {
            let server_config = parse_server(&mut tokens)?;
            config.servers.push(server_config);
        } else {
            return Err(ConfigError::ParseError(format!("Expected 'server' block, found '{}'", token)));
        }
    }
    
    Ok(config)
}

fn tokenize(content: &str) -> impl Iterator<Item = String> + '_ {
    content
        .replace("{", " { ")
        .replace("}", " } ")
        .replace(";", " ; ")
        .split_whitespace()
        .map(|s| s.to_string())
        .collect::<Vec<String>>()
        .into_iter()
}

fn parse_server(tokens: &mut impl Iterator<Item = String>) -> Result<ServerConfig, ConfigError> {
    expect_token(tokens, "{")?;
    let mut server = ServerConfig::default();
    let mut closed = false;

    while let Some(token) = tokens.next() {
        if token == "}" {
            closed = true;
            break;
        }

        match token.as_str() {
            "listen" => {
                let port_str = expect_value(tokens)?;
                let port = port_str.parse::<u16>().map_err(|_| ConfigError::ParseError(format!("Invalid port: {}", port_str)))?;
                server.ports.push(port);
                expect_token(tokens, ";")?;
            }
            "host" => {
                server.host = expect_value(tokens)?;
                expect_token(tokens, ";")?;
            }
            "server_name" => {
                while let Some(name) = tokens.next() {
                    if name == ";" { break; }
                    server.server_names.push(name);
                }
            }
            "error_page" => {
                let code_str = expect_value(tokens)?;
                let code = code_str.parse::<u16>().map_err(|_| ConfigError::ParseError(format!("Invalid status code: {}", code_str)))?;
                let path = expect_value(tokens)?;
                server.error_pages.insert(code, path);
                expect_token(tokens, ";")?;
            }
            "client_max_body_size" => {
                let size_str = expect_value(tokens)?;
                let size = size_str.parse::<usize>().map_err(|_| ConfigError::ParseError(format!("Invalid size: {}", size_str)))?;
                server.client_max_body_size = Some(size);
                expect_token(tokens, ";")?;
            }
            "timeout" => {
                let timeout_str = expect_value(tokens)?;
                let timeout = timeout_str.parse::<u64>().map_err(|_| ConfigError::ParseError(format!("Invalid timeout: {}", timeout_str)))?;
                server.timeout = Some(timeout);
                expect_token(tokens, ";")?;
            }
            "location" => {
                let location = parse_location(tokens)?;
                server.routes.push(location);
            }
            _ => return Err(ConfigError::ParseError(format!("Unknown server directive: {}", token))),
        }
    }

    if !closed {
        return Err(ConfigError::ParseError("Unclosed server block: expected '}', found EOF".to_string()));
    }
    Ok(server)
}

fn parse_location(tokens: &mut impl Iterator<Item = String>) -> Result<RouteConfig, ConfigError> {
    let mut route = RouteConfig::default();
    route.path = expect_value(tokens)?;
    expect_token(tokens, "{")?;
    let mut closed = false;

    while let Some(token) = tokens.next() {
        if token == "}" {
            closed = true;
            break;
        }

        match token.as_str() {
            "allow_methods" => {
                route.accepted_methods.clear();
                while let Some(method_str) = tokens.next() {
                    if method_str == ";" { break; }
                    match method_str.to_uppercase().as_str() {
                        "GET" => route.accepted_methods.push(HttpMethod::Get),
                        "POST" => route.accepted_methods.push(HttpMethod::Post),
                        "DELETE" => route.accepted_methods.push(HttpMethod::Delete),
                        _ => return Err(ConfigError::ParseError(format!("Unknown method: {}", method_str))),
                    }
                }
            }
            "return" => {
                route.redirection = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "root" => {
                route.root_directory = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "index" => {
                route.index_file = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "autoindex" => {
                let val = expect_value(tokens)?;
                route.directory_listing = val == "on";
                expect_token(tokens, ";")?;
            }
            "cgi_extension" => {
                route.cgi_extension = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            "upload_store" => {
                route.upload_store = Some(expect_value(tokens)?);
                expect_token(tokens, ";")?;
            }
            _ => return Err(ConfigError::ParseError(format!("Unknown location directive: {}", token))),
        }
    }

    if !closed {
        return Err(ConfigError::ParseError(format!("Unclosed location block '{}': expected '}}', found EOF", route.path)));
    }
    Ok(route)
}

fn expect_token(tokens: &mut impl Iterator<Item = String>, expected: &str) -> Result<(), ConfigError> {
    match tokens.next() {
        Some(t) if t == expected => Ok(()),
        Some(t) => Err(ConfigError::ParseError(format!("Expected '{}', found '{}'", expected, t))),
        None => Err(ConfigError::ParseError(format!("Expected '{}', found EOF", expected))),
    }
}

fn expect_value(tokens: &mut impl Iterator<Item = String>) -> Result<String, ConfigError> {
    match tokens.next() {
        Some(t) if t == ";" || t == "{" || t == "}" => Err(ConfigError::ParseError(format!("Expected value, found '{}'", t))),
        Some(t) => Ok(t),
        None => Err(ConfigError::ParseError("Expected value, found EOF".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Config {
        parse_string(src).expect("configuration valide")
    }

    fn err(src: &str) -> String {
        match parse_string(src) {
            Err(ConfigError::ParseError(m)) => m,
            Err(ConfigError::Io(e)) => panic!("erreur d'E/S inattendue : {}", e),
            Ok(_) => panic!("la configuration aurait dû être refusée : {}", src),
        }
    }

    #[test]
    fn configuration_vide_donne_zero_serveur() {
        assert!(parse("").servers.is_empty());
        assert!(parse("   \n\t  ").servers.is_empty());
    }

    #[test]
    fn bloc_minimal_avec_valeurs_par_defaut() {
        let cfg = parse("server { listen 8080; }");
        assert_eq!(cfg.servers.len(), 1);
        let s = &cfg.servers[0];
        assert_eq!(s.ports, vec![8080]);
        assert_eq!(s.host, "127.0.0.1");
        assert_eq!(s.timeout, Some(30));
        assert_eq!(s.client_max_body_size, None);
        assert!(s.routes.is_empty());
    }

    #[test]
    fn toutes_les_directives_serveur() {
        let cfg = parse(
            "server {
                listen 8080; listen 8081;
                host 0.0.0.0;
                server_name exemple.local www.exemple.local;
                error_page 404 www/404.html;
                error_page 500 www/500.html;
                client_max_body_size 1048576;
                timeout 5;
            }",
        );
        let s = &cfg.servers[0];
        assert_eq!(s.ports, vec![8080, 8081]);
        assert_eq!(s.host, "0.0.0.0");
        assert_eq!(s.server_names, vec!["exemple.local", "www.exemple.local"]);
        assert_eq!(s.error_pages.get(&404).map(String::as_str), Some("www/404.html"));
        assert_eq!(s.error_pages.get(&500).map(String::as_str), Some("www/500.html"));
        assert_eq!(s.client_max_body_size, Some(1_048_576));
        assert_eq!(s.timeout, Some(5));
    }

    #[test]
    fn toutes_les_directives_location() {
        let cfg = parse(
            "server { listen 1; location /app {
                allow_methods GET post;
                root www/app; index home.html; autoindex on;
                cgi_extension .py; upload_store uploads;
            } location /ancien { return /nouveau; } }",
        );
        let r = &cfg.servers[0].routes[0];
        assert_eq!(r.path, "/app");
        assert_eq!(r.accepted_methods, vec![HttpMethod::Get, HttpMethod::Post]);
        assert_eq!(r.root_directory.as_deref(), Some("www/app"));
        assert_eq!(r.index_file.as_deref(), Some("home.html"));
        assert!(r.directory_listing);
        assert_eq!(r.cgi_extension.as_deref(), Some(".py"));
        assert_eq!(r.upload_store.as_deref(), Some("uploads"));
        assert_eq!(cfg.servers[0].routes[1].redirection.as_deref(), Some("/nouveau"));
    }

    #[test]
    fn methodes_par_defaut_get_post_delete() {
        let r = &parse("server { listen 1; location / { root www; } }").servers[0].routes[0];
        assert_eq!(r.accepted_methods, vec![HttpMethod::Get, HttpMethod::Post, HttpMethod::Delete]);
        assert!(!r.directory_listing);
    }

    #[test]
    fn autoindex_off_et_valeur_inconnue_desactivent_le_listing() {
        let cfg = parse("server { listen 1; location /a { autoindex off; } location /b { autoindex oui; } }");
        assert!(!cfg.servers[0].routes[0].directory_listing);
        assert!(!cfg.servers[0].routes[1].directory_listing);
    }

    #[test]
    fn plusieurs_blocs_server() {
        let cfg = parse("server { listen 1; server_name a; } server { listen 2; server_name b; }");
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.servers[1].server_names, vec!["b"]);
    }

    #[test]
    fn tolere_les_espacements_compacts() {
        let cfg = parse("server{listen 8080;location /{root www;}}");
        assert_eq!(cfg.servers[0].ports, vec![8080]);
        assert_eq!(cfg.servers[0].routes[0].root_directory.as_deref(), Some("www"));
    }

    #[test]
    fn refuse_un_bloc_racine_inconnu() {
        assert!(err("http { }").contains("Expected 'server'"));
    }

    #[test]
    fn refuse_une_directive_serveur_inconnue() {
        assert!(err("server { listen 1; gzip on; }").contains("Unknown server directive: gzip"));
    }

    #[test]
    fn refuse_une_directive_location_inconnue() {
        assert!(err("server { listen 1; location / { proxy_pass x; } }").contains("Unknown location directive"));
    }

    #[test]
    fn refuse_un_port_invalide() {
        assert!(err("server { listen abc; }").contains("Invalid port"));
        assert!(err("server { listen 70000; }").contains("Invalid port"));
        assert!(err("server { listen -1; }").contains("Invalid port"));
    }

    #[test]
    fn refuse_les_valeurs_numeriques_invalides() {
        assert!(err("server { listen 1; error_page abc x; }").contains("Invalid status code"));
        assert!(err("server { listen 1; client_max_body_size 10M; }").contains("Invalid size"));
        assert!(err("server { listen 1; timeout -5; }").contains("Invalid timeout"));
    }

    #[test]
    fn refuse_une_methode_inconnue() {
        assert!(err("server { listen 1; location / { allow_methods GET PUT; } }").contains("Unknown method: PUT"));
    }

    #[test]
    fn exige_le_point_virgule() {
        assert!(err("server { listen 8080 host 0.0.0.0; }").contains("Expected ';'"));
    }

    #[test]
    fn exige_une_valeur() {
        assert!(err("server { listen ; }").contains("Expected value"));
        assert!(err("server { location { } }").contains("Expected value"));
    }

    #[test]
    fn exige_l_accolade_ouvrante() {
        assert!(err("server listen 1;").contains("Expected '{'"));
    }

    #[test]
    fn refuse_un_bloc_server_non_ferme() {
        assert!(err("server { listen 8080;").contains("Unclosed server block"));
    }

    #[test]
    fn refuse_un_bloc_location_non_ferme() {
        assert!(err("server { listen 1; location /a { root www;").contains("Unclosed location block '/a'"));
    }

    #[test]
    fn fichier_absent_donne_une_erreur_d_entree_sortie() {
        assert!(matches!(parse_file("/chemin/qui/n/existe/pas.conf"), Err(ConfigError::Io(_))));
    }

    #[test]
    fn lit_un_fichier_sur_disque() {
        let path = std::env::temp_dir().join(format!("lh-conf-{}.conf", std::process::id()));
        std::fs::write(&path, "server { listen 9000; server_name fichier; }").unwrap();
        let cfg = parse_file(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.servers[0].server_names, vec!["fichier"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn la_configuration_du_depot_est_valide() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/server.conf");
        let cfg = parse_file(path).expect("server.conf doit rester valide");
        assert!(!cfg.servers.is_empty());
        assert!(cfg.servers.iter().all(|s| !s.ports.is_empty()));
    }
}
