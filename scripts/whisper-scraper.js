// StreamNook Whisper Exporter v13
// Run this on https://www.twitch.tv/messages

(async function () {
    // Styled console logging
    const styles = {
        title: 'font-size: 16px; font-weight: bold; color: #a855f7; text-shadow: 0 0 10px #a855f755;',
        subtitle: 'font-size: 12px; color: #94a3b8;',
        step: 'font-size: 13px; font-weight: bold; color: #60a5fa;',
        success: 'font-size: 13px; color: #34d399;',
        warning: 'font-size: 12px; color: #fbbf24;',
        error: 'font-size: 13px; color: #f87171;',
        progress: 'font-size: 12px; color: #a78bfa;',
        info: 'font-size: 11px; color: #64748b;',
        highlight: 'font-size: 12px; color: #f0abfc; font-weight: bold;'
    };

    const log = (msg, style = '') => console.log(`%c${msg}`, style);
    const progressBar = (current, total, width = 20) => {
        const filled = Math.round((current / total) * width);
        const empty = width - filled;
        return `[${'█'.repeat(filled)}${'░'.repeat(empty)}] ${current}/${total}`;
    };

    // Clear console and show welcome banner
    console.clear();
    log('', '');
    log('  ✨ StreamNook Whisper Exporter', styles.title);
    log('  Your whisper history, exported safely', styles.subtitle);
    log('', '');
    log('  ─────────────────────────────────────', styles.info);
    log('  💜 This runs entirely in your browser', styles.info);
    log('  🔒 No data is sent anywhere', styles.info);
    log('  ⏱️  Takes 2-5 minutes depending on history', styles.info);
    log('  ─────────────────────────────────────', styles.info);
    log('', '');

    const exportData = {
        version: 1,
        exportedAt: new Date().toISOString(),
        myUserId: null,
        myUsername: null,
        conversations: []
    };

    const wait = ms => new Promise(r => setTimeout(r, ms));

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
            log(`  👤 Logged in as: ${exportData.myUsername}`, styles.highlight);
            log('', '');
        }
    } catch { }

    // ═══════════════════════════════════════════════════════
    // STEP 1: Initialize
    // ═══════════════════════════════════════════════════════
    log('  📂 Step 1/4: Opening whispers panel...', styles.step);

    let listEl = await openUI();
    if (!listEl) {
        log('', '');
        log('  ❌ Could not open whisper panel', styles.error);
        log('  → Make sure you\'re on twitch.tv/messages', styles.info);
        log('  → Make sure you\'re logged in', styles.info);
        return;
    }
    log('  ✓ Panel opened successfully', styles.success);
    log('', '');

    // ═══════════════════════════════════════════════════════
    // STEP 2: Find all conversations
    // ═══════════════════════════════════════════════════════
    log('  🔍 Step 2/4: Finding your conversations...', styles.step);

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

        const h = listEl.scrollHeight;
        if (Math.ceil(listEl.scrollTop + listEl.clientHeight) >= h - 50) {
            if (h === prevHeight) { noChange++; if (noChange > 6) break; }
            else noChange = 0;
        }
        prevHeight = h;
    }

    log(`  ✓ Found ${usernames.length} conversations`, styles.success);
    log('', '');

    if (usernames.length === 0) {
        log('  ⚠️ No conversations found', styles.warning);
        log('  → You may not have any whisper history', styles.info);
        return;
    }

    // ═══════════════════════════════════════════════════════
    // STEP 3: Export each conversation
    // ═══════════════════════════════════════════════════════
    log('  📥 Step 3/4: Exporting messages...', styles.step);
    log('  ⏳ Please don\'t click anywhere while this runs', styles.info);
    log('', '');

    let totalMessages = 0;
    const startTime = Date.now();

    for (let i = 0; i < usernames.length; i++) {
        const user = usernames[i];
        const progress = progressBar(i + 1, usernames.length);

        log(`  ${progress} ${user}`, styles.progress);

        listEl = await openUI();
        if (!listEl) continue;

        // Find user in list
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

        // Open conversation
        (target.closest('[class*="whispers-list-item"]') || target).click();

        // Wait for load
        for (let w = 0; w < 25; w++) {
            await wait(200);
            const h = document.querySelector('.thread-header span[title]');
            if (h?.getAttribute('title')?.toLowerCase() === user.toLowerCase()) break;
        }

        // Scrape messages
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
                                id: `msg-${user}-${messages.length}`,
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
                threadId: `thread-${user}`,
                user: { login: user.toLowerCase(), displayName: user },
                messages,
                lastMessageAt: messages[messages.length - 1].sentAt
            });
            totalMessages += messages.length;
        }

        await closeThread();
    }

    log('', '');
    log(`  ✓ Exported ${totalMessages.toLocaleString()} messages`, styles.success);

    // ═══════════════════════════════════════════════════════
    // STEP 4: Download file
    // ═══════════════════════════════════════════════════════
    log('', '');
    log('  💾 Step 4/4: Saving file...', styles.step);

    const blob = new Blob([JSON.stringify(exportData, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `streamnook_whispers_v13_${Date.now()}.json`;
    a.click();

    const elapsed = Math.round((Date.now() - startTime) / 1000);

    log('', '');
    log('  ═══════════════════════════════════════', styles.info);
    log('  ✨ Export Complete!', styles.title);
    log('  ═══════════════════════════════════════', styles.info);
    log('', '');
    log(`  📊 ${exportData.conversations.length} conversations`, styles.highlight);
    log(`  💬 ${totalMessages.toLocaleString()} messages`, styles.highlight);
    log(`  ⏱️  ${elapsed} seconds`, styles.highlight);
    log('', '');
    log('  📁 Check your Downloads folder for:', styles.info);
    log(`     streamnook_whispers_v13_*.json`, styles.progress);
    log('', '');
    log('  🎉 Now import this file in StreamNook!', styles.success);
    log('', '');
})();
