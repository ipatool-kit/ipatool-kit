//! ipatool-kit interactive menu — alternate screen + full redraw.

mod apps;
mod i18n;
mod owned_cache;
mod lists;
mod paths;
mod ui;

use std::io::Write;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ipatool_kit::client::{App, AppStoreClient, AuthInfo, DownloadRequest, LoginRequest};
use ipatool_kit::{store, HttpClient, IpatoolError, OwnedApp};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_NAME: &str = "ipatool-kit";

struct AppCtx {
    client: HttpClient,
    auth: Option<AuthInfo>,
    country: String,
}

fn main() -> ExitCode {
    let _ = paths::ensure_layout();

    let country = std::env::var("IPATOOL_COUNTRY").unwrap_or_else(|_| "us".into());
    let mut ctx = AppCtx {
        client: HttpClient::new().with_country(&country),
        auth: None,
        country,
    };
    i18n::init();

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

/// Leave alternate/raw so indicatif / install spam is visible on a normal TTY.
fn with_normal_term<T>(f: impl FnOnce() -> T) -> T {
    let mut out = std::io::stdout();
    let _ = execute!(out, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = crossterm::terminal::disable_raw_mode();
    let result = f();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = execute!(out, EnterAlternateScreen, crossterm::cursor::Hide);
    result
}

fn paint_live_status(hdr: &[String], phase: &str, secs: u64) {
    let msg = i18n::t(phase);
    let secs_s = secs.to_string();
    let body = i18n::tr(
        "status.elapsed",
        &[("msg", msg.as_str()), ("secs", secs_s.as_str())],
    );
    let _ = ui::status(hdr, &body);
}

/// Stay on the TUI screen and refresh status every ~400ms with phase + elapsed
/// so long SAP/network calls never look frozen.
fn with_live_status<T>(
    ctx: &AppCtx,
    initial: impl AsRef<str>,
    f: impl FnOnce(&mut dyn FnMut(&str)) -> T,
) -> T {
    let hdr = header(ctx);
    let phase = Arc::new(Mutex::new(initial.as_ref().to_string()));
    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();

    {
        let msg = phase.lock().map(|p| p.clone()).unwrap_or_default();
        paint_live_status(&hdr, &msg, started.elapsed().as_secs());
    }

    let phase_t = Arc::clone(&phase);
    let stop_t = Arc::clone(&stop);
    let hdr_t = hdr.clone();
    let tick = std::thread::spawn(move || {
        while !stop_t.load(Ordering::Relaxed) {
            let msg = phase_t.lock().map(|g| g.clone()).unwrap_or_default();
            paint_live_status(&hdr_t, &msg, started.elapsed().as_secs());
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    });

    let mut progress = |msg: &str| {
        if let Ok(mut g) = phase.lock() {
            *g = msg.to_string();
        }
        paint_live_status(&hdr, msg, started.elapsed().as_secs());
    };

    let result = f(&mut progress);
    stop.store(true, Ordering::Relaxed);
    let _ = tick.join();
    result
}

fn eprintln_progress(msg: &str) {
    let text = i18n::t(msg);
    let _ = writeln!(std::io::stderr(), "… {text}");
    let _ = std::io::stderr().flush();
}

fn header(ctx: &AppCtx) -> Vec<String> {
    let mut h = vec![format!("{APP_NAME} {VERSION} ({})", ctx.country)];
    if let Some(root) = paths::data_root() {
        h.push(format!("data: {}", root.display()));
    }
    match &ctx.auth {
        Some(a) => {
            h.push(
                i18n::t("header.auth_ok"),
            );
            h.push(format!("{} · {} · {}", a.email, a.name, a.storefront));
        }
        None => h.push(
            i18n::t("header.auth_none"),
        ),
    }
    h
}

fn login_screen(ctx: &mut AppCtx) -> Result<Screen, IpatoolError> {
    loop {
        let h = header(ctx);
        let items = [
            i18n::t("login.apple"),
            i18n::t("login.search_only"),
            i18n::t("common.change_language"),
            i18n::t("common.exit"),
        ];
        let i = match ui::select(&h, i18n::t("common.choose"), &items) {
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
        let items = main_items();
        let i = match ui::select(&h, i18n::t("common.choose_action"), &items)
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
                ui::message(&header(ctx), i18n::t("auth.logged_out"))?;
                return Ok(false);
            }
            15 => match apps::open_tip_jar() {
                Ok(()) => ui::message(
                    &header(ctx),
                    i18n::t("tip.opened"),
                )?,
                Err(e) => ui::error_message(
                    &header(ctx), format!(
                        "{}\nhttps://www.donationalerts.com/r/s00d88\n{e}",
                        i18n::t("tip.fallback")
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
    let term = ui::input_line(&header(ctx), i18n::t("search.app_name"))?;
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
                ui::error_message(&header(ctx), e.to_string())?;
                return Ok(());
            }
        }
    }

    if apps.is_empty() {
        ui::message(&header(ctx), i18n::t("search.none"))?;
        return Ok(());
    }

    pick_and_act(
        ctx,
        action,
        &apps,
        i18n::t("search.select"),
    )
}

fn list_then(ctx: &mut AppCtx, kind: ListKind) -> Result<(), IpatoolError> {
    let Some(dir) = lists::files_dir() else {
        ui::message(
            &header(ctx),
            i18n::t("files.missing_home"),
        )?;
        return Ok(());
    };

    let sources = [
        i18n::t("list.apple_owned"),
        i18n::t("list.downloaded_local"),
        i18n::t("list.full_apps_id"),
    ];
    let src = match ui::select(
        &header(ctx),
        i18n::t("list.which"),
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
                    i18n::t("list.need_history_cache"),
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
            i18n::t("list.empty_account"),
        )?;
        return Ok(());
    }

    let filter = ui::input_line(
        &header(ctx),
        i18n::t("list.filter"),
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
        ui::message(&header(ctx), i18n::t("list.no_matches"))?;
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
        i18n::t("list.select"),
    )
}

fn pick_and_act(
    ctx: &mut AppCtx,
    action: Action,
    apps: &[App],
    prompt: impl AsRef<str>,
) -> Result<(), IpatoolError> {
    let prompt = prompt.as_ref();
    let labels: Vec<String> = apps
        .iter()
        .map(|a| {
            if a.version.is_empty() {
                format!("{} ({})", a.name, a.id)
            } else {
                let price = if a.price == 0.0 {
                    i18n::t("search.free")
                } else {
                    format!("${:.2}", a.price)
                };
                format!("{} — {} ({}) · {}", a.name, a.version, a.id, price)
            }
        })
        .collect();

    let picked = match ui::multi_select(
        &header(ctx), format!("{prompt}  ({})", i18n::t("list.all_none")),
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
    let line = ui::input_line(&header(ctx), i18n::t("ids.prompt"))?;
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
                report.push_str(&format!("{}: {tok}\n", i18n::t("ids.invalid")));
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
        out.push_str(&i18n::t("auth.login_first"));
        return out;
    }

    match action {
        Action::Purchase => {
            match with_live_status(ctx, i18n::t("action.purchasing"), |p| {
                store::purchase_with(app.id, p)
            }) {
                Ok(()) => {
                    out.push_str(&i18n::t("action.purchased"));
                    if let Some(email) = account_email(ctx) {
                        let _ = lists::record_purchased(email, app.id, &name);
                    }
                }
                Err(e) => out.push_str(&e.to_string()),
            }
        },
        Action::DownloadLatest => {
            out.push_str(&download_one(ctx, app.id, &name, None));
        }
        Action::DownloadPickVersion => match pick_versions(ctx, app.id) {
            Ok(versions) if versions.is_empty() => {
                out.push_str(&i18n::t("action.cancelled"));
            }
            Ok(versions) => {
                for ver in versions {
                    out.push_str(&format!(
                        "  {} {} ({})\n",
                        i18n::t("action.version"),
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
        return i18n::t("download.apps_unavailable");
    };
    let tmp = apps_dir.join(format!("{app_id}.download.ipa"));
    let _ = std::fs::remove_file(&tmp);

    let _ = ui::status(
        &header(ctx), format!(
            "{}\n{name} ({app_id})",
            i18n::t("download.starting")
        ),
    );

    let result = with_normal_term(|| {
        store::download_with(
            &DownloadRequest {
                app_id: Some(app_id),
                bundle_id: None,
                output: Some(tmp.display().to_string()),
                external_version_id: external_version_id.map(str::to_string),
                purchase: true,
                keychain_passphrase: None,
            },
            &mut eprintln_progress,
            true,
        )
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
                        return i18n::t("download.missing_ipa");
                    }
                    Err(e) => return e.to_string(),
                }
            }
            format!(
                "{}\n{}",
                i18n::t("download.saved"),
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
    let ids = with_live_status(
        ctx,
        i18n::t("version.fetching_list"),
        |p| store::list_versions_with(app_id, p),
    )?;
    // Newest last in Apple dumps; show newest first for picking.
    let mut ids = ids;
    ids.reverse();

    let labels: Vec<String> = ids.iter().map(|id| format!("version id {id}")).collect();
    let pre = match ui::multi_select(
        &header(ctx),
        i18n::t("version.select_ids"),
        &labels,
    ) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };
    if pre.is_empty() {
        return Ok(Vec::new());
    }

    let mut detailed = Vec::new();
    for (n, i) in pre.iter().enumerate() {
        let id = &ids[*i];
        let label = format!(
            "{}\n({}/{}) id {id}",
            i18n::t("version.fetching_meta"),
            n + 1,
            pre.len()
        );
        let meta = with_live_status(ctx, &label, |p| {
            store::get_version_metadata_with(app_id, id, p)
        })
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
        i18n::t("version.select_download"),
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
        ui::message(&header(ctx), i18n::t("auth.login_first"))?;
        return Ok(());
    }

    let _ = ui::status(
        &header(ctx),
        i18n::t("history.preparing"),
    );

    let email = account_email(ctx).unwrap_or("").to_string();
    let mut owned: Vec<OwnedApp> = Vec::new();
    let mut from_cache = false;

    if let Some((when, cached)) = owned_cache::load_cache(&email) {
        if !cached.is_empty() {
            let label = format!(
                "{} — {} apps ({when})",
                i18n::t("history.open_cache"),
                cached.len()
            );
            let refresh = i18n::t("history.refresh");
            let items = [label.as_str(), refresh.as_str()];
            match ui::select(
                &header(ctx),
                i18n::t("history.title"),
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
        let sap_hint = if store::sap_cache_ready() {
            i18n::t("history.loading")
        } else {
            i18n::t("history.loading_sap")
        };
        let owned_res = with_live_status(ctx, &sap_hint, |p| store::list_purchases_with(p));
        owned = match owned_res {
            Ok(v) => v,
            Err(e) => {
                ui::error_message(&header(ctx), e.to_string())?;
                return Ok(());
            }
        };

        let _ = ui::status(
            &header(ctx), format!(
                "{} ({})…",
                i18n::t("history.saving_cache"),
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
            i18n::t("history.empty"),
        )?;
        return Ok(());
    }

    if !from_cache {
        if let Some(email) = account_email(ctx) {
            let _ = ui::status(
                &header(ctx), format!(
                    "{} ({})…",
                    i18n::t("history.updating_local"),
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

    let _ = ui::status(&header(ctx), i18n::t("history.building"));

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
        &header(ctx), format!(
            "{} — {}  ({})",
            i18n::t("history.apple_purchases"),
            owned.len(),
            i18n::t("list.all_none"),
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
            &header(ctx), format!(
                "{}\n{}/{}: {} ({})",
                i18n::t("download.working"),
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
        ui::message(&header(ctx), i18n::t("files.missing"))?;
        return Ok(());
    };

    let email = account_email(ctx).unwrap_or("");
    let owned_ids: std::collections::BTreeSet<i64> = owned_cache::load_cache(email)
        .map(|(_, apps)| apps.into_iter().map(|a| a.id).collect())
        .unwrap_or_default();

    if owned_ids.is_empty() {
        ui::message(
            &header(ctx),
            i18n::t("delisted.need_history"),
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
            i18n::t("delisted.none"),
        )?;
        return Ok(());
    }

    let actions = [
        i18n::t("delisted.download_latest"),
        i18n::t("delisted.download_pick"),
        i18n::t("delisted.purchase_only"),
    ];
    let act_i = match ui::select(
        &header(ctx), format!(
            "{} ({})",
            i18n::t("delisted.title"),
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
        &header(ctx), format!(
            "{} — {}  ({})",
            i18n::t("delisted.select"),
            deleted.len(),
            i18n::t("list.all_none"),
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
            &header(ctx), format!(
                "{}\n{}/{}: {} ({})",
                i18n::t("delisted.working"),
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
            &header(ctx), format!(
                "{}\n{}",
                i18n::t("apps.empty"),
                where_
            ),
        )?;
        return Ok(());
    }
    let mut report = String::new();
    report.push_str(&i18n::t("apps.min_ios"));
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
            i18n::t("apps.no_idevice"),
        )?;
        return Ok(());
    };
    let files = apps::list_ipas();
    if files.is_empty() {
        ui::message(
            &header(ctx),
            i18n::t("apps.empty"),
        )?;
        return Ok(());
    }
    let labels: Vec<String> = files
        .iter()
        .map(|f| format!("iOS {} · {}", f.meta.min_ios, f.file_name))
        .collect();
    let picked = match ui::multi_select(
        &header(ctx),
        i18n::t("apps.select_install"),
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
            i18n::t("apps.installing"),
            f.file_name
        ));
        match with_normal_term(|| apps::install_ipa(&idevice, &f.path)) {
            Ok(()) => report.push_str(&format!("{}\n", i18n::t("common.ok"))),
            Err(e) => report.push_str(&format!("{e}\n")),
        }
    }
    ui::message(&header(ctx), report.trim_end())?;
    Ok(())
}

fn clear_data(ctx: &AppCtx) -> Result<(), IpatoolError> {
    let items = [
        i18n::t("clear.purchased"),
        i18n::t("clear.downloaded"),
        i18n::t("clear.apps_folder"),
    ];
    let i = match ui::select(
        &header(ctx),
        i18n::t("clear.title"),
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
                i18n::t("clear.apps_done")
            ),
            Err(e) => e.to_string(),
        },
    };
    ui::message(&header(ctx), &msg)?;
    Ok(())
}

fn try_login(ctx: &mut AppCtx) -> Result<(), IpatoolError> {
    let email = ui::input_line(&header(ctx), i18n::t("auth.email_prompt"))?;
    if email.trim().is_empty() {
        return Ok(());
    }
    let password = ui::input_password(&header(ctx), i18n::t("auth.password_prompt"))?;
    if password.is_empty() {
        return Ok(());
    }

    let sap_hint = if store::sap_cache_ready() {
        i18n::t("auth.signing_in")
    } else {
        i18n::t("auth.signing_in_sap")
    };

    let first = with_live_status(ctx, &sap_hint, |p| {
        store::login_with(
            &LoginRequest {
                email: email.trim().into(),
                password: password.clone(),
                auth_code: None,
                keychain_passphrase: None,
            },
            p,
        )
    });

    match first {
        Ok(info) => {
            ctx.auth = Some(info);
            ui::message(&header(ctx), i18n::t("auth.login_ok"))?;
        }
        Err(e) => {
            let msg = e.to_string();
            let need_2fa = msg.to_lowercase().contains("auth-code")
                || msg.to_lowercase().contains("two-factor")
                || msg.contains("2FA")
                || msg.to_lowercase().contains("code required")
                || msg.to_lowercase().contains("verification");
            if need_2fa {
                let code = ui::input_line(&header(ctx), i18n::t("auth.twofa_prompt"))?;
                if !code.trim().is_empty() {
                    let second = with_live_status(
                        ctx,
                        i18n::t("auth.signing_in_2fa"),
                        |p| {
                            store::login_with(
                                &LoginRequest {
                                    email: email.trim().into(),
                                    password: password.clone(),
                                    auth_code: Some(code.trim().into()),
                                    keychain_passphrase: None,
                                },
                                p,
                            )
                        },
                    );
                    match second {
                        Ok(info) => {
                            ctx.auth = Some(info);
                            ui::message(
                                &header(ctx),
                                i18n::t("auth.login_ok"),
                            )?;
                        }
                        Err(e2) => ui::error_message(&header(ctx), e2.to_string())?,
                    }
                }
            } else {
                ui::error_message(&header(ctx), &msg)?;
            }
        }
    }
    Ok(())
}

fn change_language(ctx: &mut AppCtx) -> Result<(), IpatoolError> {
    let items = ["Русский", "English"];
    let i = ui::select(&header(ctx), i18n::t("common.language"), &items)?;
    let lang = if i == 0 { i18n::Lang::Ru } else { i18n::Lang::En };
    i18n::set_lang(lang);
    i18n::save_lang(lang);
    ui::message(&header(ctx), i18n::t("common.language_updated"))?;
    Ok(())
}

fn main_items() -> Vec<String> {
    i18n::main_menu_items()
}

