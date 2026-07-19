use std::io;

const REPO_OWNER: &str = "H1layd";
const REPO_NAME: &str = "Btix";
const RELEASES_URL: &str = "https://github.com/H1layd/Btix/releases";

pub fn check() -> io::Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Текущая версия: {}", current_version);
    println!("Проверяю наличие обновлений...");

    let api_url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        REPO_OWNER, REPO_NAME
    );

    let response = ureq::get(&api_url)
        .header("User-Agent", "Btix-Updater")
        .call();

    match response {
        Ok(resp) => {
            let body = resp.into_string().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, e.to_string())
            })?;

            let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, e.to_string())
            })?;

            let latest_tag = json["tag_name"]
                .as_str()
                .unwrap_or("unknown");

            // Убираем префикс "v" если он есть, для сравнения
            let latest_clean = latest_tag.trim_start_matches('v');
            let current_clean = current_version.trim_start_matches('v');

            if latest_clean == current_clean {
                println!("✓ У вас установлена актуальная версия ({})", current_version);
            } else {
                println!("⚠ Доступна новая версия: {}", latest_tag);
                println!("  Скачать можно по ссылке:");
                println!("  {}", RELEASES_URL);
                println!();
                println!("  Прямая ссылка на релиз:");
                if let Some(html_url) = json["html_url"].as_str() {
                    println!("  {}", html_url);
                }
            }

            Ok(())
        }
        Err(ureq::Error::Status(404, _)) => {
            println!("⚠ Релизы ещё не опубликованы.");
            println!("  Следите за обновлениями в репозитории:");
            println!("  https://github.com/{}/{}/", REPO_OWNER, REPO_NAME);
            Ok(())
        }
        Err(e) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Ошибка при проверке обновлений: {}", e),
        )),
    }
}