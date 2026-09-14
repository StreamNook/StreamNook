use log::debug;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

// ==============================================
// WHISPER SCRAPING AUTOMATION
// ==============================================

/// Result of whisper scraping
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperScrapeResult {
    pub success: bool,
    pub message: String,
    pub conversations: i32,
    pub messages: i32,
}

/// Whisper export data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperExportData {
    pub version: i32,
    pub exported_at: String,
    pub my_user_id: Option<String>,
    pub my_username: Option<String>,
    pub conversations: Vec<serde_json::Value>,
}

/// Start automated whisper scraping by opening a visible WebView to twitch.tv/messages
/// and running the scraper script automatically
#[cfg(desktop)]
#[tauri::command]
pub async fn scrape_whispers(app: AppHandle) -> Result<WhisperScrapeResult, String> {
    debug!("[Whisper Scraper] Starting automated whisper scraping...");

    let window_label = "whisper-scraper";
    let url = "https://www.twitch.tv/messages";

    // Close any existing scraper window
    if let Some(existing) = app.get_webview_window(window_label) {
        let _ = existing.close();
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Generate the scraper script that will send data back to Tauri
    let scraper_script = generate_whisper_scraper_script();

    debug!("[Whisper Scraper] Creating hidden window for scraping...");

    // Create a HIDDEN webview window - runs in background
    let webview_result = WebviewWindowBuilder::new(
        &app,
        window_label,
        WebviewUrl::External(url.parse().map_err(|e| format!("Invalid URL: {}", e))?),
    )
    .title("StreamNook - Importing Whispers...")
    .inner_size(600.0, 700.0)
    .center()
    .visible(false) // HIDDEN - runs silently in background
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .skip_taskbar(true) // Don't show in taskbar
    .initialization_script(&scraper_script) // Inject script on load!
    // Bind to the active account's web profile so the scraper runs against the
    // signed-in session (the same one the main login established) rather than a
    // fresh/empty default profile.
    .data_directory(
        crate::services::twitch_service::active_twitch_web_profile_dir()
            .map_err(|e| format!("Invalid web profile: {}", e))?,
    )
    .build();

    let webview = match webview_result {
        Ok(w) => w,
        Err(e) => {
            return Err(format!("Failed to create whisper scraper window: {}", e));
        }
    };

    debug!("[Whisper Scraper] Window created, script will auto-run when page loads");

    // The script will call receive_whisper_export when done
    // Return success immediately - the frontend will listen for the event
    Ok(WhisperScrapeResult {
        success: true,
        message: "Whisper scraping started. Watch the window for progress.".to_string(),
        conversations: 0,
        messages: 0,
    })
}

/// Progress event for whisper scraping
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperProgress {
    pub step: i32,
    pub status: String,
    pub detail: String,
    pub current: i32,
    pub total: i32,
}

/// Emit whisper scraping progress to the frontend
#[tauri::command]
pub async fn emit_whisper_progress(
    app: AppHandle,
    step: i32,
    status: String,
    detail: String,
    current: i32,
    total: i32,
) -> Result<(), String> {
    let progress = WhisperProgress {
        step,
        status,
        detail,
        current,
        total,
    };
    let _ = app.emit("whisper-import-progress", &progress);
    Ok(())
}

/// Receive whisper export data from the WebView scraper script
#[cfg(desktop)]
#[tauri::command]
pub async fn receive_whisper_export(
    app: AppHandle,
    data: WhisperExportData,
) -> Result<WhisperScrapeResult, String> {
    debug!("[Whisper Scraper] Received export data from WebView!");
    debug!(
        "[Whisper Scraper] {} conversations, exported at {}",
        data.conversations.len(),
        data.exported_at
    );

    let total_messages: i32 = data
        .conversations
        .iter()
        .map(|c| {
            c.get("messages")
                .and_then(|m| m.as_array())
                .map(|arr| arr.len() as i32)
                .unwrap_or(0)
        })
        .sum();

    // Save the data to a file in the app's data directory
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let whispers_dir = app_data_dir.join("whispers");

    // Create directory if it doesn't exist
    if !whispers_dir.exists() {
        std::fs::create_dir_all(&whispers_dir)
            .map_err(|e| format!("Failed to create whispers dir: {}", e))?;
    }

    // Save the export file
    let filename = format!(
        "whisper_export_{}.json",
        chrono::Utc::now().format("%Y%m%d_%H%M%S")
    );
    let file_path = whispers_dir.join(&filename);

    let json_data =
        serde_json::to_string_pretty(&data).map_err(|e| format!("Failed to serialize: {}", e))?;

    std::fs::write(&file_path, &json_data).map_err(|e| format!("Failed to write file: {}", e))?;

    debug!("[Whisper Scraper] Saved to {:?}", file_path);

    // Close the scraper window
    if let Some(window) = app.get_webview_window("whisper-scraper") {
        let _ = window.close();
    }

    // Emit success event to the frontend
    let result = WhisperScrapeResult {
        success: true,
        message: format!("Successfully imported {} whispers!", total_messages),
        conversations: data.conversations.len() as i32,
        messages: total_messages,
    };

    let _ = app.emit("whisper-import-complete", &result);

    // Also emit the actual data for the frontend to process
    let _ = app.emit("whisper-data-ready", &data);

    // Focus the main window
    if let Some(main_window) = app.get_webview_window("main") {
        let _ = main_window.set_focus();
    }

    Ok(result)
}

/// Generate the whisper scraper script that sends data back to Tauri with progress events
fn generate_whisper_scraper_script() -> String {
    // This is the modified scraper that emits progress events and calls Tauri invoke when done
    r#"
    // StreamNook Whisper Exporter - In-App Version with Progress Events
    (async function() {
        // Wait for page to fully load
        await new Promise(r => setTimeout(r, 2000));
        
        console.log('%c✨ StreamNook Whisper Importer', 'font-size:16px;font-weight:bold;color:#a855f7');
        
        const exportData = {
            version: 1,
            exportedAt: new Date().toISOString(),
            myUserId: null,
            myUsername: null,
            conversations: []
        };
        
        const wait = ms => new Promise(r => setTimeout(r, ms));
        
        // Helper to emit progress events to the frontend
        const emitProgress = (step, status, detail, current, total) => {
            window.__TAURI_INTERNALS__.invoke('emit_whisper_progress', {
                step, status, detail, current, total
            }).catch(() => {});
        };
        
        // Find the whisper button
        const getWhisperNavButton = () => {
            const path = document.querySelector('path[d="M9.828 17 12 19.172 14.172 17H19V5H5v12h4.828ZM12 22l-3-3H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-4l-3 3Z"]');
            return path ? (path.closest('button') || path.closest('[role="button"]')) : document.querySelector('[data-a-target="whisper-box-button"]');
        };
        
        // Find conversation list
        const getList = () => {
            for (const el of document.querySelectorAll('.scrollable-area')) {
                if (el.querySelector('[class*="whispers-list-item"]')) return el;
            }
            return null;
        };
        
        // Ensure whisper UI is open
        const openUI = async () => {
            let list = getList();
            if (list?.offsetParent) return list;
            const btn = getWhisperNavButton();
            if (!btn) return null;
            btn.click();
            await wait(1500);
            list = getList();
            if (list?.offsetParent) return list;
            btn.click();
            await wait(1500);
            return getList();
        };
        
        // Close open thread
        const closeThread = async () => {
            const btn = document.querySelector('button[aria-label="Close"][data-a-target^="thread-close-button"]');
            if (btn) { btn.click(); await wait(800); return true; }
            return false;
        };
        
        // Detect current user
        try {
            const userMenu = document.querySelector('[data-a-target="user-menu-toggle"]');
            const img = userMenu?.querySelector('img');
            if (img?.alt) {
                exportData.myUsername = img.alt.replace("'s Avatar", '').replace("'s avatar", '');
            }
        } catch {}
        
        // STEP 1: Open panel
        emitProgress(1, 'running', 'Opening whispers panel...', 0, 4);
        let listEl = await openUI();
        if (!listEl) {
            emitProgress(1, 'error', 'Could not open whisper panel. Make sure you are logged in!', 0, 4);
            window.__TAURI_INTERNALS__.invoke('receive_whisper_export', { 
                data: { ...exportData, error: 'Could not open whisper panel' }
            });
            return;
        }
        emitProgress(1, 'complete', 'Panel opened', 1, 4);
        
        // STEP 2: Find conversations
        emitProgress(2, 'running', 'Scanning for conversations...', 1, 4);
        const usernames = [];
        await closeThread();
        listEl.scrollTop = 0;
        await wait(500);
        
        let prevHeight = 0, noChange = 0;
        for (let i = 0; i < 200; i++) {
            document.querySelectorAll('[class*="whispers-list-item__user-name"]').forEach(el => {
                const name = el.getAttribute('title') || el.textContent.trim();
                if (name && !usernames.includes(name)) usernames.push(name);
            });
            listEl.scrollTop += 600;
            await wait(250);
            // Update progress periodically
            if (i % 10 === 0) {
                emitProgress(2, 'running', `Found ${usernames.length} conversations so far...`, 1, 4);
            }
            const h = listEl.scrollHeight;
            if (Math.ceil(listEl.scrollTop + listEl.clientHeight) >= h - 50) {
                if (h === prevHeight) { noChange++; if (noChange > 6) break; }
                else noChange = 0;
            }
            prevHeight = h;
        }
        
        emitProgress(2, 'complete', `Found ${usernames.length} conversations`, 2, 4);
        
        if (usernames.length === 0) {
            emitProgress(3, 'complete', 'No conversations to export', 4, 4);
            window.__TAURI_INTERNALS__.invoke('receive_whisper_export', { data: exportData });
            return;
        }
        
        // STEP 3: Export messages
        emitProgress(3, 'running', 'Starting message export...', 2, 4);
        
        let totalMessages = 0;
        
        for (let i = 0; i < usernames.length; i++) {
            const user = usernames[i];
            emitProgress(3, 'running', `Exporting: ${user}`, i, usernames.length);
            
            listEl = await openUI();
            if (!listEl) continue;
            
            let target = null;
            listEl.scrollTop = 0;
            await wait(100);
            
            for (let s = 0; s < 80 && !target; s++) {
                for (const el of document.querySelectorAll('[class*="whispers-list-item__user-name"]')) {
                    if ((el.getAttribute('title') || el.textContent.trim()) === user) {
                        target = el;
                        break;
                    }
                }
                if (!target) {
                    listEl.scrollTop += 500;
                    await wait(80);
                    if (listEl.scrollTop + listEl.clientHeight >= listEl.scrollHeight - 10) break;
                }
            }
            
            if (!target) continue;
            
            (target.closest('[class*="whispers-list-item"]') || target).click();
            
            for (let w = 0; w < 25; w++) {
                await wait(200);
                const h = document.querySelector('.thread-header span[title]');
                if (h?.getAttribute('title')?.toLowerCase() === user.toLowerCase()) break;
            }
            
            const messages = [];
            const threadBox = document.querySelector('.whispers-thread-messages__thread-box');
            const scroll = threadBox?.querySelector('.scrollable-area');
            
            if (scroll) {
                scroll.scrollTop = 0;
                await wait(300);
                scroll.scrollTop = scroll.scrollHeight;
                await wait(200);
                
                const sample = scroll.querySelector('[data-a-target="whisper-message"]');
                const container = sample?.parentElement;
                
                if (container) {
                    let timestamp = new Date().toLocaleDateString();
                    
                    for (const child of container.children) {
                        if (child.classList.contains('thread-message__timestamp') || child.querySelector('.thread-message__timestamp')) {
                            const span = child.querySelector('span[title]');
                            if (span) timestamp = span.getAttribute('title');
                        }
                        
                        const msgEl = child.getAttribute('data-a-target') === 'whisper-message'
                            ? child
                            : child.querySelector('[data-a-target="whisper-message"]');
                        
                        if (msgEl) {
                            const nameEl = msgEl.querySelector('[data-a-target="whisper-message-name"]');
                            const textEl = msgEl.querySelector('.text-fragment, [data-a-target="chat-message-text"]');
                            
                            if (nameEl && textEl) {
                                const from = nameEl.getAttribute('aria-label') || nameEl.textContent.trim();
                                messages.push({
                                    id: 'msg-' + user + '-' + messages.length,
                                    fromUserName: from,
                                    content: textEl.textContent.trim(),
                                    sentAt: timestamp,
                                    isSent: exportData.myUsername && from.toLowerCase() === exportData.myUsername.toLowerCase()
                                });
                            }
                        }
                    }
                }
            }
            
            if (messages.length > 0) {
                exportData.conversations.push({
                    threadId: 'thread-' + user,
                    user: { login: user.toLowerCase(), displayName: user },
                    messages,
                    lastMessageAt: messages[messages.length - 1].sentAt
                });
                totalMessages += messages.length;
            }
            
            await closeThread();
        }
        
        emitProgress(3, 'complete', `Exported ${totalMessages} messages from ${exportData.conversations.length} conversations`, 3, 4);
        
        // STEP 4: Send to StreamNook
        emitProgress(4, 'running', 'Finalizing import...', 3, 4);
        
        try {
            await window.__TAURI_INTERNALS__.invoke('receive_whisper_export', { data: exportData });
            emitProgress(4, 'complete', `Import complete! ${totalMessages} messages`, 4, 4);
        } catch (e) {
            emitProgress(4, 'error', 'Failed to send data: ' + e.message, 4, 4);
        }
    })();
    "#.to_string()
}
