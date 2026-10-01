//! ipatool-kit interactive menu — alternate screen + full redraw.

mod apps;
mod owned_cache;
mod lists;
mod paths;
mod ui;

use std::process::ExitCode;

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ipatool::client::{App, AppStoreClient, AuthInfo, DownloadRequest, LoginRequest};
use ipatool::{store, HttpClient, IpatoolError, OwnedApp};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_NAME: &str = "ipatool-kit";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Ru,
}

struct AppCtx {
    lang: Lang,
    client: HttpClient,
    auth: Option<AuthInfo>,
    country: String,
}

fn main() -> ExitCode {
    let _ = paths::ensure_layout();

    let country = std::env::var("IPATOOL_COUNTRY").unwrap_or_else(|_| "us".into());
    let mut ctx = AppCtx {
        lang: load_lang(),
        client: HttpClient::new().with_country(&country),
        auth: None,
        country,
    };
    apply_ui_lang(ctx.lang);

    if let Ok(info) = store::auth_info() {
        if !info.email.is_empty() {
            ctx.auth = Some(info);
        }
    }

    let _guard = match ui::TermGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    match run(&mut ctx) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            drop(_guard);
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(ctx: &mut AppCtx) -> Result<(), IpatoolError> {
    loop {
        if ctx.auth.is_none() {
            match login_screen(ctx)? {
                Screen::Quit => return Ok(()),
                Screen::Main => {}
            }
        }
        if main_screen(ctx)? {
            return Ok(());
        }
    }
}

enum Screen {
    Main,
    Quit,
}

/// Leave alternate/raw so download/login progress / terminal spam is visible.
fn with_normal_term<T>(f: impl FnOnce() -> T) -> T {
    let mut out = std::io::stdout();
    let _ = execute!(out, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = crossterm::terminal::disable_raw_mode();
    let result = f();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = execute!(out, EnterAlternateScreen, crossterm::cursor::Hide);
    result
}

fn header(ctx: &AppCtx) -> Vec<String> {
    let mut h = vec![format!("{APP_NAME} {VERSION} ({})", ctx.country)];
    if let Some(root) = paths::data_root() {
        h.push(format!("data: {}", root.display()));
    }
    match &ctx.auth {
        Some(a) => {
            h.push(
                t(
                    ctx,
                    "Apple account login successful.",
                    "Вход в аккаунт Apple выполнен.",
                )
                .into(),
            );
            h.push(format!("{} · {} · {}", a.email, a.name, a.storefront));
        }
        None => h.push(
            t(
                ctx,
                "Not authenticated / search-only.",
                "Нет входа / только поиск.",
            )
            .into(),
        ),
    }
    h
}

fn login_screen(ctx: &mut AppCtx) -> Result<Screen, IpatoolError> {
    loop {
        let h = header(ctx);
        let items = [
            t(ctx, "Log in to Apple account", "Войти в аккаунт Apple"),
            t(
                ctx,
                "Continue in search-only mode",
                "Продолжить только с поиском",
            ),
            t(ctx, "Change language", "Сменить язык"),
            t(ctx, "Exit", "Выход"),
        ];
        let i = match ui::select(&h, t(ctx, "Choose", "Выберите"), &items) {
            Ok(v) => v,
            Err(_) => return Ok(Screen::Quit),
        };
        match i {
            0 => {
                try_login(ctx)?;
                if ctx.auth.is_some() {
                    return Ok(Screen::Main);
                }
            }
            1 => return Ok(Screen::Main),
            2 => change_language(ctx)?,
            _ => return Ok(Screen::Quit),
        }
    }
}

fn main_screen(ctx: &mut AppCtx) -> Result<bool, IpatoolError> {
    loop {
        let h = header(ctx);
        let items = main_items(ctx);
        let i = match ui::select(&h, t(ctx, "Choose an action", "Выберите действие"), &items)
        {
            Ok(v) => v,
            Err(_) => return Ok(true),
        };
        match i {
            0 => search_then(ctx, Action::Purchase)?,
            1 => search_then(ctx, Action::DownloadLatest)?,
            2 => search_then(ctx, Action::DownloadPickVersion)?,
            3 => ids_then(ctx, Action::Purchase)?,
            4 => ids_then(ctx, Action::DownloadLatest)?,
            5 => ids_then(ctx, Action::DownloadPickVersion)?,
            6..=8 => {
                let kind = match i {
                    6 => ListKind::Purchase,
                    7 => ListKind::DownloadLatest,
                    _ => ListKind::DownloadPickVersion,
                };
                list_then(ctx, kind)?;
            }
            9 => purchase_history_then(ctx)?,
            10 => deleted_apps_then(ctx)?,
            11 => apps_min_ios(ctx)?,
            12 => apps_install(ctx)?,
            13 => clear_data(ctx)?,
            14 => {
                let _ = store::revoke();
                ctx.auth = None;
                ui::message(&header(ctx), t(ctx, "Logged out.", "Выход выполнен."))?;
                return Ok(false);
            }
            15 => match apps::open_tip_jar() {
                Ok(()) => ui::message(
                    &header(ctx),
                    t(
                        ctx,
                        "Opened DonationAlerts — thank you!",
                        "Открыл DonationAlerts — спасибо!",
                    ),
                )?,
                Err(e) => ui::message(
                    &header(ctx),
                    &format!(
                        "{}\nhttps://www.donationalerts.com/r/s00d88\n{e}",
                        t(
                            ctx,
                            "Could not open browser. Tip link:",
                            "Не открылся браузер. Ссылка:",
                        )
                    ),
                )?,
            },
            16 => change_language(ctx)?,
            _ => return Ok(true),
        }
    }
}

#[derive(Clone, Copy)]
enum Action {
    Purchase,
    DownloadLatest,
    DownloadPickVersion,
}

#[derive(Clone, Copy)]
enum ListKind {
    Purchase,
    DownloadLatest,
    DownloadPickVersion,
}

fn account_email(ctx: &AppCtx) -> Option<&str> {
    ctx.auth.as_ref().map(|a| a.email.as_str())
}

fn search_then(ctx: &mut AppCtx, action: Action) -> Result<(), IpatoolError> {
    let term = ui::input_line(&header(ctx), t(ctx, "App name: ", "Название: "))?;
    let term = term.trim();
    if term.is_empty() {
        return Ok(());
    }

    let mut apps: Vec<App> = lists::search_local(term, account_email(ctx))
        .into_iter()
        .map(|a| App {
            id: a.id,
            name: a.name,
            ..App::default()
        })
        .collect();
    let mut seen: std::collections::BTreeSet<i64> = apps.iter().map(|a| a.id).collect();

    match ctx.client.search(term, 10, None) {
        Ok(sr) => {
            for a in sr.results {
                if seen.insert(a.id) {
                    apps.push(a);
                }
            }
        }
        Err(e) => {
            if apps.is_empty() {
                ui::message(&header(ctx), &e.to_string())?;
                return Ok(());
            }
        }
    }

    if apps.is_empty() {
        ui::message(&header(ctx), t(ctx, "No apps found.", "Ничего не найдено."))?;
        return Ok(());
    }

    pick_and_act(
        ctx,
        action,
        &apps,
        t(ctx, "Select apps", "Выберите приложения"),
    )
}

fn list_then(ctx: &mut AppCtx, kind: ListKind) -> Result<(), IpatoolError> {
    let Some(dir) = lists::files_dir() else {
        ui::message(
            &header(ctx),
            t(
                ctx,
                "Data Files/ not found. Set IPA_DOWNLOADER_HOME.",
                "Нет Files/. Задай IPA_DOWNLOADER_HOME.",
            ),
        )?;
        return Ok(());
    };

    let sources = [
        t(
            ctx,
            "Apple purchase history (all owned)",
            "История покупок Apple (все мои)",
        ),
        t(ctx, "My downloaded (local)", "Мои скачанные (локально)"),
        t(
            ctx,
            "Full Apps_ID_List (GitHub)",
            "Полный Apps_ID_List (GitHub)",
        ),
    ];
    let src = match ui::select(
        &header(ctx),
        t(ctx, "Which list?", "Какой список?"),
        &sources,
    ) {
        Ok(i) => i,
        Err(_) => return Ok(()),
    };

    let email = account_email(ctx);
    let listed = match src {
        0 => {
            // Prefer Apple owned-apps cache (incl. hidden / removed-from-device).
            let email_s = email.unwrap_or("");
            if let Some((_, owned)) = owned_cache::load_cache(email_s) {
                owned
                    .into_iter()
                    .map(|a| lists::ListedApp {
                        id: a.id,
                        name: if a.name.is_empty() {
                            a.bundle_id
                        } else {
                            a.name
                        },
                    })
                    .collect()
            } else {
                ui::message(
                    &header(ctx),
                    t(
                        ctx,
                        "No Apple history cache yet.\nOpen «Purchase history» menu first to load it.",
                        "Нет кэша истории Apple.\nСначала открой пункт «История покупок».",
                    ),
                )?;
                return Ok(());
            }
        }
        1 => lists::history_for_account(&lists::downloaded(&dir), email),
        _ => lists::load_apps_id_list(&dir),
    };

    if listed.is_empty() {
        ui::message(
            &header(ctx),
            t(
                ctx,
                "List is empty for this account.",
                "Список пуст для этого аккаунта.",
            ),
        )?;
        return Ok(());
    }

    let filter = ui::input_line(
        &header(ctx),
        t(ctx, "Filter (empty = all): ", "Фильтр (пусто = все): "),
    )?;
    let filter = filter.trim().to_lowercase();
    let listed: Vec<_> = if filter.is_empty() {
        listed
    } else {
        listed
            .into_iter()
            .filter(|a| a.name.to_lowercase().contains(&filter))
            .collect()
    };
    if listed.is_empty() {
        ui::message(&header(ctx), t(ctx, "No matches.", "Нет совпадений."))?;
        return Ok(());
    }

    let apps: Vec<App> = listed
        .into_iter()
        .map(|a| App {
            id: a.id,
            name: a.name,
            ..App::default()
        })
        .collect();

    let action = match kind {
        ListKind::Purchase => Action::Purchase,
        ListKind::DownloadLatest => Action::DownloadLatest,
        ListKind::DownloadPickVersion => Action::DownloadPickVersion,
    };
    pick_and_act(
        ctx,
        action,
        &apps,
        t(ctx, "Select apps from list", "Выберите из списка"),
    )
}

fn pick_and_act(
    ctx: &mut AppCtx,
    action: Action,
    apps: &[App],
    prompt: &str,
) -> Result<(), IpatoolError> {
    let labels: Vec<String> = apps
        .iter()
        .map(|a| {
            if a.version.is_empty() {
                format!("{} ({})", a.name, a.id)
            } else {
                let price = if a.price == 0.0 {
                    t(ctx, "Free", "Бесплатно").to_string()
                } else {
                    format!("${:.2}", a.price)
                };
                format!("{} — {} ({}) · {}", a.name, a.version, a.id, price)
            }
        })
        .collect();

    let picked = match ui::multi_select(
        &header(ctx),
        &format!("{prompt}  ({})", t(ctx, "* = all/none", "* = всё/снять")),
        &labels,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if picked.is_empty() {
        return Ok(());
    }

    let mut report = String::new();
    for idx in picked {
        report.push_str(&apply_action(ctx, action, &apps[idx]));
        report.push('\n');
    }
    ui::message(&header(ctx), report.trim_end())?;
    Ok(())
}

fn ids_then(ctx: &mut AppCtx, action: Action) -> Result<(), IpatoolError> {
    let line = ui::input_line(&header(ctx), t(ctx, "App IDs: ", "ID приложений: "))?;
    if line.trim().is_empty() {
        return Ok(());
    }
    let mut report = String::new();
    for tok in line.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        match tok.parse::<i64>() {
            Ok(id) => {
                let name = lists::resolve_name(id, "");
                let stub = App {
                    id,
                    name,
                    ..App::default()
                };
                report.push_str(&apply_action(ctx, action, &stub));
                report.push('\n');
            }
            Err(_) => {
                report.push_str(&format!("{}: {tok}\n", t(ctx, "Invalid ID", "Неверный ID")));
            }
        }
    }
    ui::message(&header(ctx), report.trim_end())?;
    Ok(())
}

fn apply_action(ctx: &AppCtx, action: Action, app: &App) -> String {
    let name = lists::resolve_name(app.id, &app.name);
    let mut out = format!("→ {name} ({})\n", app.id);
    if ctx.auth.is_none() {
        out.push_str(t(ctx, "Log in first.", "Сначала войди."));
        return out;
    }

    match action {
        Action::Purchase => match with_normal_term(|| store::purchase(app.id)) {
            Ok(()) => {
                out.push_str(t(ctx, "purchased", "куплено"));
                if let Some(email) = account_email(ctx) {
                    let _ = lists::record_purchased(email, app.id, &name);
                }
            }
            Err(e) => out.push_str(&e.to_string()),
        },
        Action::DownloadLatest => {
            out.push_str(&download_one(ctx, app.id, &name, None));
        }
        Action::DownloadPickVersion => match pick_versions(ctx, app.id) {
            Ok(versions) if versions.is_empty() => {
                out.push_str(t(ctx, "cancelled", "отменено"));
            }
            Ok(versions) => {
                for ver in versions {
                    out.push_str(&format!(
                        "  {} {} ({})\n",
                        t(ctx, "version", "версия"),
                        ver.display_version,
                        ver.external_id
                    ));
                    out.push_str(&download_one(
                        ctx,
                        app.id,
                        &name,
                        Some(ver.external_id.as_str()),
                    ));
                    out.push('\n');
                }
            }
            Err(e) => out.push_str(&e.to_string()),
        },
    }
    out
}

fn download_one(
    ctx: &AppCtx,
    app_id: i64,
    name: &str,
    external_version_id: Option<&str>,
) -> String {
    let Some(apps_dir) = apps::apps_dir() else {
        return t(ctx, "Apps/ not available.", "Apps/ недоступна.").into();
    };
    let tmp = apps_dir.join(format!("{app_id}.download.ipa"));
    let _ = std::fs::remove_file(&tmp);

    let result = with_normal_term(|| {
        store::download(&DownloadRequest {
            app_id: Some(app_id),
            bundle_id: None,
            output: Some(tmp.display().to_string()),
            external_version_id: external_version_id.map(str::to_string),
            purchase: true,
            keychain_passphrase: None,
        })
        .map(|_| ())
    });
    match result {
        Ok(()) => {
            // Also scoop any leftover cwd dumps from older ipatool defaults.
            let mut from = vec![apps_dir.clone()];
            if let Ok(cwd) = std::env::current_dir() {
                from.push(cwd);
            }
            let mut lines = Vec::new();
            if tmp.exists() {
                match apps::finalize_downloaded_ipa(&tmp, app_id, name, account_email(ctx)) {
                    Ok(r) => lines.push(r),
                    Err(e) => return e.to_string(),
                }
            } else {
                match apps::ingest_downloaded_ipas(app_id, name, account_email(ctx), &from) {
                    Ok(r) if !r.is_empty() => lines.extend(r),
                    Ok(_) => {
                        return t(
                            ctx,
                            "download ok but .ipa not found",
                            "скачано, но .ipa не найден",
                        )
                        .into();
                    }
                    Err(e) => return e.to_string(),
                }
            }
            format!(
                "{}\n{}",
                t(ctx, "Saved to Apps/:", "Сохранено в Apps/:"),
                lines.join("\n")
            )
        }
        Err(e) => e.to_string(),
    }
}

#[derive(Clone)]
struct VersionMeta {
    external_id: String,
    display_version: String,
}

fn pick_versions(
    ctx: &AppCtx,
    app_id: i64,
) -> Result<Vec<VersionMeta>, IpatoolError> {
    ui::message(
        &header(ctx),
        t(
            ctx,
            "Loading version ID list…",
            "Загрузка списка ID версий…",
        ),
    )?;
    let ids = with_normal_term(|| store::list_versions(app_id))?;
    // Newest last in Apple dumps; show newest first for picking.
    let mut ids = ids;
    ids.reverse();

    let labels: Vec<String> = ids.iter().map(|id| format!("version id {id}")).collect();
    let pre = match ui::multi_select(
        &header(ctx),
        t(
            ctx,
            "Select version IDs to inspect (Space)",
            "Выберите ID версий для просмотра (Space)",
        ),
        &labels,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };
    if pre.is_empty() {
        return Ok(Vec::new());
    }

    ui::message(
        &header(ctx),
        t(
            ctx,
            "Fetching version metadata…",
            "Загрузка метаданных версий…",
        ),
    )?;
    let mut detailed = Vec::new();
    for i in &pre {
        let id = &ids[*i];
        let meta = with_normal_term(|| store::get_version_metadata(app_id, id))
            .map(|v| VersionMeta {
                external_id: v.external_id,
                display_version: if v.display_version.is_empty() {
                    "NA".into()
                } else {
                    v.display_version
                },
            })
            .unwrap_or(VersionMeta {
                external_id: id.clone(),
                display_version: "NA".into(),
            });
        detailed.push(meta);
    }

    let labels: Vec<String> = detailed
        .iter()
        .map(|m| format!("{}  ({})", m.display_version, m.external_id))
        .collect();
    let final_pick = match ui::multi_select(
        &header(ctx),
        t(
            ctx,
            "Select versions to download",
            "Выберите версии для скачивания",
        ),
        &labels,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    Ok(final_pick
        .into_iter()
        .map(|i| detailed[i].clone())
        .collect())
}

fn purchase_history_then(ctx: &mut AppCtx) -> Result<(), IpatoolError> {
    if ctx.auth.is_none() {
        ui::message(&header(ctx), t(ctx, "Log in first.", "Сначала войди."))?;
        return Ok(());
    }

    let _ = ui::status(
        &header(ctx),
        t(
            ctx,
            "Preparing purchase history…",
            "Готовлю историю покупок…",
        ),
    );

    let email = account_email(ctx).unwrap_or("").to_string();
    let mut owned: Vec<OwnedApp> = Vec::new();
    let mut from_cache = false;

    if let Some((when, cached)) = owned_cache::load_cache(&email) {
        if !cached.is_empty() {
            let label = format!(
                "{} — {} apps ({when})",
                t(ctx, "Open cached list now", "Открыть кэш сразу"),
                cached.len()
            );
            let items = [
                label.as_str(),
                t(
                    ctx,
                    "Refresh from Apple (~30–60 sec)",
                    "Обновить из Apple (~30–60 сек)",
                ),
            ];
            match ui::select(
                &header(ctx),
                t(ctx, "Purchase history", "История покупок"),
                &items,
            ) {
                Ok(0) => {
                    owned = cached;
                    from_cache = true;
                }
                Ok(_) => {}
                Err(_) => return Ok(()),
            }
        }
    }

    if owned.is_empty() {
        let _ = ui::status(
            &header(ctx),
            t(
                ctx,
                "Loading Apple purchase history…\n(first run may download SAP runtime)",
                "Загрузка истории покупок Apple…\n(первый запуск может скачать SAP runtime)",
            ),
        );
        let owned_res = with_normal_term(store::list_purchases);
        owned = match owned_res {
            Ok(v) => v,
            Err(e) => {
                ui::message(&header(ctx), &e.to_string())?;
                return Ok(());
            }
        };

        let _ = ui::status(
            &header(ctx),
            &format!(
                "{} ({})…",
                t(ctx, "Saving cache", "Сохраняю кэш"),
                owned.len()
            ),
        );
        if !email.is_empty() {
            let _ = owned_cache::save_cache(&email, &owned);
        }
    }

    if owned.is_empty() {
        ui::message(
            &header(ctx),
            t(ctx, "Purchase history is empty.", "История покупок пуста."),
        )?;
        return Ok(());
    }

    if !from_cache {
        if let Some(email) = account_email(ctx) {
            let _ = ui::status(
                &header(ctx),
                &format!(
                    "{} ({})…",
                    t(
                        ctx,
                        "Updating local purchased list",
                        "Обновляю локальный список покупок"
                    ),
                    owned.len()
                ),
            );
            let rows: Vec<(i64, String)> = owned
                .iter()
                .map(|a| {
                    let name = if a.name.is_empty() {
                        a.bundle_id.clone()
                    } else {
                        a.name.clone()
                    };
                    (a.id, name)
                })
                .collect();
            let _ = lists::record_purchased_bulk(email, &rows);
        }
    }

    let _ = ui::status(&header(ctx), t(ctx, "Building list…", "Собираю список…"));

    let labels: Vec<String> = owned
        .iter()
        .map(|a| {
            let date = if a.purchase_date.is_empty() {
                String::new()
            } else {
                a.purchase_date.chars().take(10).collect::<String>()
            };
            let ver = if a.version.is_empty() {
                String::new()
            } else {
                format!(" · {}", a.version)
            };
            let plat = if a.platforms.is_empty() {
                String::new()
            } else {
                format!(" [{}]", a.platforms.join(","))
            };
            format!(
                "{} ({}){ver}{plat}  {date}",
                if a.name.is_empty() {
                    &a.bundle_id
                } else {
                    &a.name
                },
                a.id
            )
        })
        .collect();

    let picked = match ui::multi_select(
        &header(ctx),
        &format!(
            "{} — {}  ({})",
            t(ctx, "Apple purchases", "Покупки Apple"),
            owned.len(),
            t(ctx, "* = all/none", "* = всё/снять"),
        ),
        &labels,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if picked.is_empty() {
        return Ok(());
    }

    let mut report = String::new();
    let total_pick = picked.len();
    for (n, idx) in picked.into_iter().enumerate() {
        let a = &owned[idx];
        let _ = ui::status(
            &header(ctx),
            &format!(
                "{}\n{}/{}: {} ({})",
                t(ctx, "Downloading…", "Скачивание…"),
                n + 1,
                total_pick,
                if a.name.is_empty() {
                    &a.bundle_id
                } else {
                    &a.name
                },
                a.id
            ),
        );
        let app = App {
            id: a.id,
            name: if a.name.is_empty() {
                a.bundle_id.clone()
            } else {
                a.name.clone()
            },
            version: a.version.clone(),
            ..App::default()
        };
        report.push_str(&apply_action(ctx, Action::DownloadLatest, &app));
        report.push('\n');
    }
    ui::message(&header(ctx), report.trim_end())?;
    Ok(())
}

/// Apps in Apps_ID_List that are missing from Apple purchase-history cache (delisted etc.).
fn deleted_apps_then(ctx: &mut AppCtx) -> Result<(), IpatoolError> {
    let Some(dir) = lists::files_dir() else {
        ui::message(&header(ctx), t(ctx, "Files/ missing.", "Нет Files/."))?;
        return Ok(());
    };

    let email = account_email(ctx).unwrap_or("");
    let owned_ids: std::collections::BTreeSet<i64> = owned_cache::load_cache(email)
        .map(|(_, apps)| apps.into_iter().map(|a| a.id).collect())
        .unwrap_or_default();

    if owned_ids.is_empty() {
        ui::message(
            &header(ctx),
            t(
                ctx,
                "Load Apple purchase history first (menu item above),\nso we can subtract it from Apps_ID_List.",
                "Сначала загрузи историю покупок Apple (пункт выше),\nчтобы вычесть её из Apps_ID_List.",
            ),
        )?;
        return Ok(());
    }

    let deleted: Vec<lists::ListedApp> = lists::load_apps_id_list(&dir)
        .into_iter()
        .filter(|a| a.id != 0 && !owned_ids.contains(&a.id))
        .collect();

    if deleted.is_empty() {
        ui::message(
            &header(ctx),
            t(
                ctx,
                "No delisted-only apps: everything in Apps_ID_List is already in Apple history.",
                "Нет «только удалённых»: всё из Apps_ID_List уже есть в истории Apple.",
            ),
        )?;
        return Ok(());
    }

    let actions = [
        t(ctx, "Download latest", "Скачать latest"),
        t(
            ctx,
            "Download with version pick",
            "Скачать с выбором версии",
        ),
        t(ctx, "Purchase only", "Только покупка"),
    ];
    let act_i = match ui::select(
        &header(ctx),
        &format!(
            "{} ({})",
            t(
                ctx,
                "Deleted / not in Apple history",
                "Удалённые / нет в истории Apple"
            ),
            deleted.len()
        ),
        &actions,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let action = match act_i {
        0 => Action::DownloadLatest,
        1 => Action::DownloadPickVersion,
        _ => Action::Purchase,
    };

    let labels: Vec<String> = deleted
        .iter()
        .map(|a| format!("{} ({})", a.name, a.id))
        .collect();

    let picked = match ui::multi_select(
        &header(ctx),
        &format!(
            "{} — {}  ({})",
            t(ctx, "Delisted apps", "Снятые / удалённые"),
            deleted.len(),
            t(ctx, "* = all/none", "* = всё/снять"),
        ),
        &labels,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if picked.is_empty() {
        return Ok(());
    }

    let apps: Vec<App> = picked
        .into_iter()
        .map(|i| App {
            id: deleted[i].id,
            name: deleted[i].name.clone(),
            ..App::default()
        })
        .collect();

    let mut report = String::new();
    for (n, app) in apps.iter().enumerate() {
        let _ = ui::status(
            &header(ctx),
            &format!(
                "{}\n{}/{}: {} ({})",
                t(ctx, "Working…", "Работаю…"),
                n + 1,
                apps.len(),
                app.name,
                app.id
            ),
        );
        report.push_str(&apply_action(ctx, action, app));
        report.push('\n');
    }
    ui::message(&header(ctx), report.trim_end())?;
    Ok(())
}

fn apps_min_ios(ctx: &AppCtx) -> Result<(), IpatoolError> {
    let files = apps::list_ipas();
    if files.is_empty() {
        let where_ = apps::apps_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Apps/".into());
        ui::message(
            &header(ctx),
            &format!(
                "{}\n{}",
                t(
                    ctx,
                    "No apps found in Apps folder.",
                    "В папке Apps нет приложений.",
                ),
                where_
            ),
        )?;
        return Ok(());
    }
    let mut report = String::new();
    report.push_str(t(ctx, "Min. iOS version:", "Мин. версия iOS:"));
    report.push('\n');
    for (i, f) in files.iter().enumerate() {
        report.push_str(&format!(
            "{:>3}.  iOS {:>5}  {}\n",
            i + 1,
            f.meta.min_ios,
            f.file_name
        ));
    }
    ui::message(&header(ctx), report.trim_end())?;
    Ok(())
}

fn apps_install(ctx: &AppCtx) -> Result<(), IpatoolError> {
    let Some(idevice) = apps::find_ideviceinstaller() else {
        ui::message(
            &header(ctx),
            t(
                ctx,
                "ideviceinstaller not found. USB install unavailable (AirDrop still works on macOS).",
                "ideviceinstaller не найден. USB-установка недоступна (на macOS остаётся AirDrop).",
            ),
        )?;
        return Ok(());
    };
    let files = apps::list_ipas();
    if files.is_empty() {
        ui::message(
            &header(ctx),
            t(
                ctx,
                "No apps found in Apps folder.",
                "В папке Apps нет приложений.",
            ),
        )?;
        return Ok(());
    }
    let labels: Vec<String> = files
        .iter()
        .map(|f| format!("iOS {} · {}", f.meta.min_ios, f.file_name))
        .collect();
    let picked = match ui::multi_select(
        &header(ctx),
        t(
            ctx,
            "Select apps to install",
            "Выберите приложения для установки",
        ),
        &labels,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if picked.is_empty() {
        return Ok(());
    }

    let mut report = String::new();
    for i in picked {
        let f = &files[i];
        report.push_str(&format!(
            "{} {}\n",
            t(ctx, "Installing:", "Установка:"),
            f.file_name
        ));
        match with_normal_term(|| apps::install_ipa(&idevice, &f.path)) {
            Ok(()) => report.push_str(&format!("{}\n", t(ctx, "ok", "ок"))),
            Err(e) => report.push_str(&format!("{e}\n")),
        }
    }
    ui::message(&header(ctx), report.trim_end())?;
    Ok(())
}

fn clear_data(ctx: &AppCtx) -> Result<(), IpatoolError> {
    let items = [
        t(ctx, "Purchased apps list", "Список купленных"),
        t(ctx, "Downloaded apps list", "Список скачанных"),
        t(ctx, "Apps in Apps folder", "Приложения в Apps/"),
    ];
    let i = match ui::select(
        &header(ctx),
        t(ctx, "Select data to clear", "Что очистить?"),
        &items,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let email = account_email(ctx);
    let msg = match i {
        0 => match lists::clear_history("Purchased_IDs.json", email) {
            Ok(s) => s,
            Err(e) => e,
        },
        1 => match lists::clear_history("Downloaded_IDs.json", email) {
            Ok(s) => s,
            Err(e) => e,
        },
        _ => match apps::clear_all_ipas() {
            Ok(n) => format!(
                "{} ({n})",
                t(ctx, "Apps folder cleared", "Папка Apps очищена")
            ),
            Err(e) => e.to_string(),
        },
    };
    ui::message(&header(ctx), &msg)?;
    Ok(())
}

fn try_login(ctx: &mut AppCtx) -> Result<(), IpatoolError> {
    let email = ui::input_line(&header(ctx), t(ctx, "Email: ", "Email: "))?;
    if email.trim().is_empty() {
        return Ok(());
    }
    let password = ui::input_password(&header(ctx), t(ctx, "Password: ", "Пароль: "))?;
    if password.is_empty() {
        return Ok(());
    }

    let _ = ui::status(
        &header(ctx),
        t(
            ctx,
            "Signing in… (first run may download SAP runtime)",
            "Вход… (первый запуск может скачать SAP runtime)",
        ),
    );

    let first = with_normal_term(|| {
        store::login(&LoginRequest {
            email: email.trim().into(),
            password: password.clone(),
            auth_code: None,
            keychain_passphrase: None,
        })
    });

    match first {
        Ok(info) => {
            ctx.auth = Some(info);
            ui::message(&header(ctx), t(ctx, "Login successful.", "Вход выполнен."))?;
        }
        Err(e) => {
            let msg = e.to_string();
            let need_2fa = msg.to_lowercase().contains("auth-code")
                || msg.to_lowercase().contains("two-factor")
                || msg.contains("2FA")
                || msg.to_lowercase().contains("code required")
                || msg.to_lowercase().contains("verification");
            if need_2fa {
                let code = ui::input_line(&header(ctx), t(ctx, "2FA code: ", "Код 2FA: "))?;
                if !code.trim().is_empty() {
                    let second = with_normal_term(|| {
                        store::login(&LoginRequest {
                            email: email.trim().into(),
                            password: password.clone(),
                            auth_code: Some(code.trim().into()),
                            keychain_passphrase: None,
                        })
                    });
                    match second {
                        Ok(info) => {
                            ctx.auth = Some(info);
                            ui::message(
                                &header(ctx),
                                t(ctx, "Login successful.", "Вход выполнен."),
                            )?;
                        }
                        Err(e2) => ui::message(&header(ctx), &e2.to_string())?,
                    }
                }
            } else {
                ui::message(&header(ctx), &msg)?;
            }
        }
    }
    Ok(())
}

fn change_language(ctx: &mut AppCtx) -> Result<(), IpatoolError> {
    let items = ["Русский", "English"];
    let i = ui::select(&header(ctx), t(ctx, "Language", "Язык"), &items)?;
    ctx.lang = if i == 0 { Lang::Ru } else { Lang::En };
    apply_ui_lang(ctx.lang);
    save_lang(ctx.lang);
    ui::message(&header(ctx), t(ctx, "Language updated.", "Язык обновлён."))?;
    Ok(())
}

fn apply_ui_lang(lang: Lang) {
    ui::set_lang(matches!(lang, Lang::Ru));
}

fn load_lang() -> Lang {
    if let Some(root) = paths::data_root() {
        let p = root.join("lang");
        if let Ok(s) = std::fs::read_to_string(p) {
            let s = s.trim().to_ascii_lowercase();
            if s.starts_with("ru") || s == "russian" || s == "русский" {
                return Lang::Ru;
            }
            if s.starts_with("en") || s == "english" {
                return Lang::En;
            }
        }
    }
    // Default from system locale when no saved preference.
    let loc = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if loc.starts_with("ru") {
        Lang::Ru
    } else {
        Lang::En
    }
}

fn save_lang(lang: Lang) {
    if let Some(root) = paths::data_root() {
        let tag = match lang {
            Lang::Ru => "ru\n",
            Lang::En => "en\n",
        };
        let _ = std::fs::write(root.join("lang"), tag);
    }
}

fn main_items(ctx: &AppCtx) -> Vec<&'static str> {
    match ctx.lang {
        Lang::En => vec![
            "Search for app and purchase (without downloading)",
            "Search for app and download latest version",
            "Search for app and download (with version selection)",
            "Enter app IDs and purchase (without downloading)",
            "Enter app IDs and download latest version",
            "Enter app IDs and download (with version selection)",
            "Show list of apps and purchase (without downloading)",
            "Show list of apps and download latest version",
            "Show list of apps and download (with version selection)",
            "Purchase history: Apple owned apps and download",
            "Deleted/delisted apps (Apps_ID_List not in Apple history)",
            "Check minimum iOS version for apps in Apps folder",
            "Install apps from Apps folder",
            "Clear data",
            "Log out of Apple account and reset settings",
            "Tip Jar",
            "Change language",
            "Exit",
        ],
        Lang::Ru => vec![
            "Поиск приложения и покупка (без скачивания)",
            "Поиск приложения и скачивание последней версии",
            "Поиск приложения и скачивание (с выбором версии)",
            "Ввод ID и покупка (без скачивания)",
            "Ввод ID и скачивание последней версии",
            "Ввод ID и скачивание (с выбором версии)",
            "Список приложений и покупка (без скачивания)",
            "Список приложений и скачивание последней версии",
            "Список приложений и скачивание (с выбором версии)",
            "История покупок Apple и скачать",
            "Удалённые/снятые со стора (нет в истории Apple)",
            "Проверить min iOS для Apps/",
            "Установить из Apps/",
            "Очистить данные",
            "Выйти из аккаунта и сбросить настройки",
            "Чаевые (DonationAlerts)",
            "Сменить язык",
            "Выход",
        ],
    }
}

fn t<'a>(ctx: &AppCtx, en: &'a str, ru: &'a str) -> &'a str {
    match ctx.lang {
        Lang::En => en,
        Lang::Ru => ru,
    }
}
