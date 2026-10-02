import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open as openDialog } from '@tauri-apps/plugin-dialog';

document.addEventListener('DOMContentLoaded', async () => {
    const navItems = document.querySelectorAll('.nav-item');
    const views = document.querySelectorAll('.view');
    const viewHome = document.getElementById('view-home');

    const urlInput = document.getElementById('url-input');
    const urlFeedback = document.getElementById('url-feedback');
    const searchInputBox = document.querySelector('.search-input-box');
    const clearBtn = document.getElementById('clear-btn');
    const downloadMainBtn = document.getElementById('download-main-btn');
    const dlBadge = document.getElementById('dl-badge');
    const downloadsList = document.getElementById('downloads-list');

    const navSettingsBtn = document.getElementById('nav-settings');
    const closeSettingsBtn = document.getElementById('close-settings-btn');
    const settingsDrawer = document.getElementById('settings-drawer');
    const drawerOverlay = document.getElementById('drawer-overlay');
    const checkAllUpdatesBtn = document.getElementById('check-all-updates-btn');

    const trimSliderStart = document.getElementById('trim-slider-start');
    const trimSliderEnd = document.getElementById('trim-slider-end');
    const trimHighlight = document.getElementById('trim-highlight');
    const trimInputStart = document.getElementById('trim-input-start');
    const trimInputEnd = document.getElementById('trim-input-end');

    const depModal = document.getElementById('dep-modal');
    const modalTitle = document.getElementById('modal-dep-title');
    const modalDesc = document.getElementById('modal-dep-desc');
    const modalSource = document.getElementById('modal-dep-source');
    const modalProgress = document.getElementById('modal-dep-progress');
    const modalStatus = document.getElementById('modal-dep-status');
    const modalPercent = document.getElementById('modal-dep-percent');
    const modalBar = document.getElementById('modal-dep-bar');
    const modalActions = document.getElementById('modal-dep-actions');
    const modalCancelBtn = document.getElementById('modal-cancel-btn');
    const modalActionBtn = document.getElementById('modal-action-btn');

    let maxVideoDuration = 0;
    let currentMetadata = null;
    let downloads = {};
    let appSettings = { download_path: '', last_update_check: null };
    let cachedDeps = {
        yt_dlp: { installed: false, update_available: false },
        ffmpeg: { installed: false, update_available: false },
        js_runtime: { installed: false, update_available: false },
    };

    let modalActiveTool = null;
    let modalAction = null;
    let modalCallback = null;
    let dependencyOperationRunning = false;

    function escapeHtml(value) {
        return String(value ?? '').replace(/[&<>"']/g, (char) => ({
            '&': '&amp;',
            '<': '&lt;',
            '>': '&gt;',
            '"': '&quot;',
            "'": '&#039;',
        })[char]);
    }

    function safeImageUrl(value) {
        try {
            const parsed = new URL(String(value));
            if (parsed.protocol === 'https:' || parsed.protocol === 'http:') {
                return parsed.href;
            }
        } catch (_) {
        }
        return '';
    }

    function showUrlError(message) {
        urlFeedback.textContent = message;
        urlFeedback.classList.add('visible');
        searchInputBox.classList.add('invalid');
    }

    function clearUrlError() {
        urlFeedback.textContent = '';
        urlFeedback.classList.remove('visible');
        searchInputBox.classList.remove('invalid');
    }

    function validateUrl(value) {
        if (!value) return 'Paste a video or audio link first.';
        let parsed;
        try {
            parsed = new URL(value);
        } catch (_) {
            return 'Enter a complete link, such as https://www.youtube.com/watch?v=…';
        }
        if (!['http:', 'https:'].includes(parsed.protocol) || !parsed.hostname) {
            return 'Only web links beginning with http:// or https:// are supported.';
        }
        if (parsed.username || parsed.password) {
            return 'Remove the username or password from this link and try again.';
        }
        return null;
    }

    function mediaErrorMessage(error) {
        const lines = String(error || '').split(/\r?\n/).map((line) => line.trim());
        const detail = lines.find((line) => line.startsWith('ERROR:')) || lines.find(Boolean) || '';
        if (/unsupported url/i.test(detail)) return 'This site or link is not supported by yt-dlp.';
        if (/private|sign in|login/i.test(detail)) return 'This media may be private or require sign-in.';
        if (/unavailable|not found|404/i.test(detail)) return 'This media is unavailable. Check the link and try again.';
        return detail.replace(/^ERROR:\s*/, '').slice(0, 260) || 'Could not read this link. Check it and try again.';
    }

    function displayHeight(height) {
        const actual = Number(height);
        const standard = [144, 240, 360, 480, 720, 1080, 1440, 2160, 4320]
            .find((value) => Math.abs(value - actual) <= 3);
        return standard || actual;
    }

    function syncOverlay() {
        const shouldShow =
            settingsDrawer.classList.contains('active') ||
            depModal.classList.contains('active');
        drawerOverlay.classList.toggle('active', shouldShow);
    }

    function switchView(viewId) {
        if (!viewId) return;

        settingsDrawer.classList.remove('active');
        syncOverlay();

        navItems.forEach((item) => {
            item.classList.toggle('active', item.getAttribute('data-view') === viewId);
        });

        views.forEach((view) => {
            view.classList.toggle('active', view.id === viewId);
        });
        window.scrollTo(0, 0);
    }

    navItems.forEach((item) => {
        item.addEventListener('click', (event) => {
            event.preventDefault();
            const viewId = item.getAttribute('data-view');
            if (viewId) switchView(viewId);
        });
    });

    function formatBytes(bytes) {
        const numeric = Number(bytes);
        if (!Number.isFinite(numeric) || numeric <= 0) return 'Size unavailable';

        const units = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
        const index = Math.min(
            Math.floor(Math.log(numeric) / Math.log(1024)),
            units.length - 1,
        );
        return `${(numeric / Math.pow(1024, index)).toFixed(index === 0 ? 0 : 2)} ${units[index]}`;
    }

    function renderDependencyCard(toolKey, info) {
        const badge = document.getElementById(`badge-${toolKey}`);
        const installedEl = document.getElementById(`${toolKey}-installed-ver`);
        const latestEl = document.getElementById(`${toolKey}-latest-ver`);
        const actionsContainer = document.getElementById(`actions-${toolKey}`);

        installedEl.textContent = info.installed_version || 'Not installed';
        const latestDisplay = toolKey === 'ffmpeg' && info.latest_version?.includes('T')
            ? info.latest_version.slice(0, 10)
            : (info.latest_version || '—');
        latestEl.textContent = latestDisplay;
        document.getElementById(`${toolKey}-install-path`).textContent = info.installed
            ? info.install_path
            : 'Not installed';

        badge.className = 'dep-badge';
        actionsContainer.innerHTML = '';

        if (toolKey === 'deno' && info.status === 'System runtime') {
            badge.textContent = 'Available';
            badge.classList.add('status-ok');
            return;
        }

        if (!info.installed) {
            badge.textContent = 'Not installed';
            badge.classList.add('status-missing');
            actionsContainer.innerHTML = `
                <button class="btn-compact btn-compact-accent" id="install-btn-${toolKey}">Install</button>
            `;

            document.getElementById(`install-btn-${toolKey}`).addEventListener('click', () => {
                runDependencyAction(toolKey, 'install');
            });
            return;
        }

        if (info.update_available) {
            badge.textContent = 'Update available';
            badge.classList.add('status-update');
            actionsContainer.innerHTML = `
                <button class="btn-compact btn-compact-accent" id="update-btn-${toolKey}">Update</button>
                <button class="btn-compact btn-compact-subtle-delete" id="delete-btn-${toolKey}" title="Delete Link's managed copy">Delete</button>
            `;

            document.getElementById(`update-btn-${toolKey}`).addEventListener('click', () => {
                runDependencyAction(toolKey, 'update');
            });
            document.getElementById(`delete-btn-${toolKey}`).addEventListener('click', () => {
                deleteDependency(toolKey);
            });
            return;
        }

        badge.textContent = info.status === 'Up to date' ? 'Up to date' : 'Installed';
        badge.classList.add('status-ok');
        actionsContainer.innerHTML = `
            <button class="btn-compact btn-compact-outline" id="check-btn-${toolKey}">Check</button>
            <button class="btn-compact btn-compact-subtle-delete" id="delete-btn-${toolKey}" title="Delete Link's managed copy">Delete</button>
        `;

        document.getElementById(`check-btn-${toolKey}`).addEventListener('click', checkForUpdates);
        document.getElementById(`delete-btn-${toolKey}`).addEventListener('click', () => {
            deleteDependency(toolKey);
        });
    }

    async function refreshDependencyState() {
        cachedDeps = await invoke('get_dependency_status');
        renderDependencyCard('ytdlp', cachedDeps.yt_dlp);
        renderDependencyCard('ffmpeg', cachedDeps.ffmpeg);
        renderDependencyCard('deno', cachedDeps.js_runtime);
        return cachedDeps;
    }

    async function checkForUpdates() {
        const oldText = checkAllUpdatesBtn.textContent;
        checkAllUpdatesBtn.textContent = 'Checking…';
        checkAllUpdatesBtn.disabled = true;

        try {
            cachedDeps = await invoke('check_dependency_updates');
            renderDependencyCard('ytdlp', cachedDeps.yt_dlp);
            renderDependencyCard('ffmpeg', cachedDeps.ffmpeg);
            renderDependencyCard('deno', cachedDeps.js_runtime);
            appSettings.last_update_check = Math.floor(Date.now() / 1000);
        } catch (error) {
            console.error('Failed to check dependency updates:', error);
            alert(`Could not check for updates:\n${error}`);
        } finally {
            checkAllUpdatesBtn.textContent = oldText || 'Check for updates';
            checkAllUpdatesBtn.disabled = false;
        }
    }

    checkAllUpdatesBtn.addEventListener('click', checkForUpdates);

    async function deleteDependency(toolKey) {
        const displayName = toolKey === 'ytdlp' ? 'yt-dlp' : toolKey === 'ffmpeg' ? 'FFmpeg' : 'Deno';
        const shouldDelete = window.confirm(
            `Remove ${displayName} from Link?\n\nThis only deletes Link's managed copy.`,
        );

        if (!shouldDelete) return;

        try {
            await invoke(toolKey === 'ytdlp' ? 'remove_ytdlp' : toolKey === 'ffmpeg' ? 'remove_ffmpeg' : 'remove_deno');
            await refreshDependencyState();
        } catch (error) {
            alert(`Failed to remove ${displayName}:\n${error}`);
        }
    }

    function showToolModal(toolKey, action, onDone = null, requiredForDownload = false) {
        modalActiveTool = toolKey;
        modalAction = action;
        modalCallback = onDone;
        dependencyOperationRunning = false;

        const isYtdlp = toolKey === 'ytdlp';
        const displayName = isYtdlp ? 'yt-dlp' : toolKey === 'ffmpeg' ? 'FFmpeg' : 'Deno';
        const actionWord = action === 'update' ? 'Update' : 'Install';

        if (requiredForDownload) {
            modalTitle.textContent = isYtdlp
                ? 'yt-dlp is required'
                : toolKey === 'deno' ? 'A JavaScript runtime is required for YouTube' : 'FFmpeg is required for this download';
            modalDesc.textContent = isYtdlp
                ? 'yt-dlp is required to inspect media links and retrieve available formats.'
                : toolKey === 'deno'
                    ? 'Deno lets yt-dlp read all available YouTube formats. Link installs a verified copy for this app.'
                    : 'FFmpeg is required to merge streams, create MP3 audio, or trim this download.';
        } else {
            modalTitle.textContent = `${actionWord} ${displayName}`;
            modalDesc.textContent = action === 'update'
                ? `Link found a newer trusted ${displayName} build. The current working copy is kept until the new download is verified.`
                : `Link will download and verify ${displayName} before activating it.`;
        }

        modalSource.innerHTML = isYtdlp
            ? '<strong>Source:</strong> Official yt-dlp/yt-dlp GitHub release · SHA-256 verified'
            : toolKey === 'deno'
                ? '<strong>Source:</strong> Official denoland/deno GitHub release · SHA-256 verified'
                : '<strong>Source:</strong> BtbN/FFmpeg-Builds Windows x64 static build · linked by FFmpeg.org · SHA-256 verified';

        modalActionBtn.textContent = `${actionWord} ${displayName}`;
        modalCancelBtn.disabled = false;
        modalActionBtn.disabled = false;
        modalProgress.classList.remove('active');
        modalActions.style.display = 'flex';
        modalBar.style.width = '0%';
        modalPercent.textContent = '0%';
        modalStatus.textContent = 'Ready';

        depModal.classList.add('active');
        syncOverlay();
    }

    function hideToolModal() {
        if (dependencyOperationRunning) return;

        depModal.classList.remove('active');
        modalActiveTool = null;
        modalAction = null;
        modalCallback = null;
        syncOverlay();
    }

    function runDependencyAction(toolKey, action, onDone = null, requiredForDownload = false) {
        showToolModal(toolKey, action, onDone, requiredForDownload);
    }

    modalCancelBtn.addEventListener('click', hideToolModal);

    modalActionBtn.addEventListener('click', async () => {
        if (!modalActiveTool || !modalAction || dependencyOperationRunning) return;

        const commandMap = {
            ytdlp: {
                install: 'install_ytdlp',
                update: 'update_ytdlp',
            },
            ffmpeg: {
                install: 'install_ffmpeg',
                update: 'update_ffmpeg',
            },
            deno: {
                install: 'install_deno',
                update: 'update_deno',
            },
        };

        const command = commandMap[modalActiveTool]?.[modalAction];
        if (!command) return;

        dependencyOperationRunning = true;
        modalActions.style.display = 'none';
        modalProgress.classList.add('active');
        modalStatus.textContent = 'Starting…';
        modalPercent.textContent = '0%';
        modalBar.style.width = '0%';

        try {
            await invoke(command);
            await refreshDependencyState();

            modalStatus.textContent = 'Installed';
            modalPercent.textContent = '100%';
            modalBar.style.width = '100%';

            const callback = modalCallback;
            dependencyOperationRunning = false;

            setTimeout(() => {
                depModal.classList.remove('active');
                modalActiveTool = null;
                modalAction = null;
                modalCallback = null;
                syncOverlay();
                if (callback) callback();
            }, 450);
        } catch (error) {
            dependencyOperationRunning = false;
            modalActions.style.display = 'flex';
            modalProgress.classList.remove('active');
            alert(`Installation/update failed:\n${error}`);
        }
    });

    await listen('dependency-progress', (event) => {
        const { tool, phase, percentage } = event.payload;

        if (modalActiveTool && tool === modalActiveTool) {
            modalStatus.textContent = phase;
            modalPercent.textContent = `${percentage}%`;
            modalBar.style.width = `${percentage}%`;
        }
    });

    async function initSettings() {
        try {
            appSettings = await invoke('get_settings');
            document.getElementById('setting-path').textContent = appSettings.download_path;
            document.getElementById('setting-path').title = appSettings.download_path;
            await refreshDependencyState();

            const now = Math.floor(Date.now() / 1000);
            const lastCheck = Number(appSettings.last_update_check || 0);

            if (!lastCheck || now - lastCheck >= 86400) {
                invoke('check_dependency_updates')
                    .then((deps) => {
                        cachedDeps = deps;
                        appSettings.last_update_check = Math.floor(Date.now() / 1000);
                        renderDependencyCard('ytdlp', deps.yt_dlp);
                        renderDependencyCard('ffmpeg', deps.ffmpeg);
                        renderDependencyCard('deno', deps.js_runtime);
                    })
                    .catch((error) => {
                        console.warn('Background dependency update check failed:', error);
                    });
            }
        } catch (error) {
            console.error('Could not initialize Link settings:', error);
        }
    }

    document.getElementById('change-path-btn').addEventListener('click', async () => {
        const selected = await openDialog({
            directory: true,
            multiple: false,
            defaultPath: appSettings.download_path,
        });

        if (selected) {
            appSettings.download_path = selected;
            document.getElementById('setting-path').textContent = selected;
            document.getElementById('setting-path').title = selected;
            await invoke('save_settings', { path: selected });
        }
    });

    urlInput.addEventListener('input', () => {
        clearUrlError();
        currentMetadata = null;
        viewHome.classList.remove('has-results');
        const hasValue = urlInput.value.trim().length > 0;
        clearBtn.classList.toggle('visible', hasValue);
        downloadMainBtn.disabled = !hasValue;
    });

    clearBtn.addEventListener('click', () => {
        urlInput.value = '';
        urlInput.dispatchEvent(new Event('input'));
        urlInput.focus();
        viewHome.classList.remove('has-results');
        currentMetadata = null;
    });

    async function performMetadataFetch() {
        const url = urlInput.value.trim();
        const validationError = validateUrl(url);
        if (validationError) {
            showUrlError(validationError);
            urlInput.focus();
            return;
        }
        const host = new URL(url).hostname.toLowerCase();
        const isYouTube = host === 'youtu.be' || host === 'youtube.com' || host.endsWith('.youtube.com');
        if (isYouTube && !cachedDeps.js_runtime.installed) {
            runDependencyAction('deno', 'install', performMetadataFetch, true);
            return;
        }

        const originalButton = downloadMainBtn.innerHTML;
        downloadMainBtn.disabled = true;
        downloadMainBtn.innerHTML = `
            <svg class="spinner" viewBox="0 0 24 24">
                <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" stroke-dasharray="32" fill="none"></circle>
            </svg>`;

        try {
            currentMetadata = await invoke('fetch_metadata', { url });

            document.getElementById('meta-title').textContent = currentMetadata.title || 'Unknown Title';
            document.getElementById('meta-thumb').src = safeImageUrl(currentMetadata.thumbnail);
            document.getElementById('meta-uploader').textContent =
                currentMetadata.uploader || currentMetadata.channel || 'Unknown';
            document.getElementById('meta-duration').textContent = formatTime(currentMetadata.duration || 0);

            const dateContainer = document.getElementById('meta-date-container');
            if (currentMetadata.upload_date) {
                dateContainer.style.display = 'flex';
                document.getElementById('meta-date').textContent = currentMetadata.upload_date;
            } else {
                dateContainer.style.display = 'none';
            }

            maxVideoDuration = Math.max(0, Math.floor(Number(currentMetadata.duration) || 0));
            trimSliderStart.max = maxVideoDuration;
            trimSliderEnd.max = maxVideoDuration;
            trimSliderStart.value = 0;
            trimSliderEnd.value = maxVideoDuration;
            updateTrimVisuals();

            renderFormats(currentMetadata.formats || []);
            viewHome.classList.add('has-results');
        } catch (error) {
            showUrlError(mediaErrorMessage(error));
        } finally {
            downloadMainBtn.innerHTML = originalButton;
            downloadMainBtn.disabled = urlInput.value.trim().length === 0;
        }
    }

    downloadMainBtn.addEventListener('click', () => {
        const validationError = validateUrl(urlInput.value.trim());
        if (validationError) {
            showUrlError(validationError);
            urlInput.focus();
            return;
        }
        if (!cachedDeps.yt_dlp.installed) {
            runDependencyAction('ytdlp', 'install', performMetadataFetch, true);
        } else {
            performMetadataFetch();
        }
    });

    function renderFormats(formats) {
        const videoGrid = document.getElementById('video-formats');
        const audioGrid = document.getElementById('audio-formats');
        videoGrid.innerHTML = '';
        audioGrid.innerHTML = '';

        const videoFormats = formats.filter((format) =>
            format.vcodec &&
            format.vcodec !== 'none' &&
            Number(format.height) > 0
        );

        const byHeight = new Map();
        videoFormats.forEach((format) => {
            const height = displayHeight(format.height);
            const existing = byHeight.get(height);
            const hasAudio = Boolean(format.acodec && format.acodec !== 'none');
            const existingHasAudio = Boolean(existing?.acodec && existing.acodec !== 'none');
            const isMp4 = String(format.ext || '').toLowerCase() === 'mp4';
            const existingIsMp4 = String(existing?.ext || '').toLowerCase() === 'mp4';
            const score = Number(isMp4) * 2 + Number(hasAudio);
            const existingScore = Number(existingIsMp4) * 2 + Number(existingHasAudio);

            if (
                !existing ||
                score > existingScore ||
                (score === existingScore &&
                    (format.filesize || format.filesize_approx || 0) >
                    (existing.filesize || existing.filesize_approx || 0))
            ) {
                byHeight.set(height, format);
            }
        });

        [...byHeight.values()]
            .sort((a, b) => (b.height || 0) - (a.height || 0))
            .forEach((format, index) => {
                const size = formatBytes(format.filesize || format.filesize_approx);
                const label = format.height ? `${displayHeight(format.height)}p` : (format.resolution || 'Video');
                const hasAudio = Boolean(format.acodec && format.acodec !== 'none');
                const sourceExt = String(format.ext || '').toLowerCase();

                videoGrid.insertAdjacentHTML('beforeend', `
                    <div class="format-card req-dl-btn"
                         title="Source height: ${escapeHtml(format.height)} pixels"
                         data-id="${escapeHtml(format.format_id)}"
                         data-type="video"
                         data-label="${escapeHtml(label)}"
                         data-size="${escapeHtml(size)}"
                         data-has-audio="${hasAudio}"
                         data-source-ext="${escapeHtml(sourceExt)}">
                        ${index === 0 ? '<div class="badge-recommended">Best</div>' : ''}
                        <div class="format-card-header">
                            <span class="format-quality">${escapeHtml(label)}</span>
                            <span class="format-type">MP4</span>
                        </div>
                        <div class="format-card-footer">
                            <span class="format-size">${escapeHtml(size)}</span>
                            <div class="format-download-icon">
                                <svg viewBox="0 0 24 24"><line x1="12" y1="5" x2="12" y2="19"></line><polyline points="19 12 12 19 5 12"></polyline></svg>
                            </div>
                        </div>
                    </div>`);
            });

        const audioFormats = formats
            .filter((format) =>
                format.vcodec === 'none' &&
                format.acodec &&
                format.acodec !== 'none'
            )
            .sort((a, b) => (b.abr || 0) - (a.abr || 0))
            .slice(0, 3);

        audioFormats.forEach((format, index) => {
            const size = formatBytes(format.filesize || format.filesize_approx);
            const label = format.abr ? `${Math.round(format.abr)}k` : 'Best audio';

            audioGrid.insertAdjacentHTML('beforeend', `
                <div class="format-card req-dl-btn"
                     data-id="${escapeHtml(format.format_id)}"
                     data-type="audio"
                     data-label="${escapeHtml(label)}"
                     data-size="${escapeHtml(size)}"
                     data-has-audio="true">
                    ${index === 0 ? '<div class="badge-recommended">HQ</div>' : ''}
                    <div class="format-card-header">
                        <span class="format-quality">${escapeHtml(label)}</span>
                        <span class="format-type">MP3</span>
                    </div>
                    <div class="format-card-footer">
                        <span class="format-size">${escapeHtml(size)}</span>
                        <div class="format-download-icon">
                            <svg viewBox="0 0 24 24"><line x1="12" y1="5" x2="12" y2="19"></line><polyline points="19 12 12 19 5 12"></polyline></svg>
                        </div>
                    </div>
                </div>`);
        });

        document.querySelectorAll('.req-dl-btn').forEach((button) => {
            button.addEventListener('click', () => handleFormatSelection(button.dataset));
        });
    }

    document.querySelectorAll('.tab-btn').forEach((button) => {
        button.addEventListener('click', () => {
            document.querySelectorAll('.tab-btn').forEach((tab) => tab.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach((content) => content.classList.remove('active'));
            button.classList.add('active');
            document.getElementById(button.dataset.target).classList.add('active');
        });
    });

    function handleFormatSelection(dataset) {
        const isTrimming =
            Number(trimSliderStart.value) > 0 ||
            Number(trimSliderEnd.value) < maxVideoDuration;
        const isAudioExtraction = dataset.type === 'audio';
        const selectedVideoAlreadyHasAudio = dataset.hasAudio === 'true';
        const needsMerge = dataset.type === 'video' && !selectedVideoAlreadyHasAudio;
        const needsConversion = dataset.type === 'video' && dataset.sourceExt !== 'mp4';
        const needsFFmpeg = isTrimming || isAudioExtraction || needsMerge || needsConversion;

        if (needsFFmpeg && !cachedDeps.ffmpeg.installed) {
            runDependencyAction(
                'ffmpeg',
                'install',
                () => initiateDownload(dataset),
                true,
            );
        } else {
            initiateDownload(dataset);
        }
    }

    function updateBadge() {
        const count = Object.values(downloads).filter((download) => download.state === 'downloading').length;
        dlBadge.textContent = count;
        dlBadge.classList.toggle('active', count > 0);
    }

    function markDownloadFailed(id, error) {
        const download = downloads[id];
        const element = document.getElementById(id);
        if (!download || !element) return;

        download.state = 'failed';
        element.classList.remove('state-downloading');
        element.classList.add('state-failed');
        element.querySelector('.dl-status-text').textContent = 'Failed';
        const speedElement = document.getElementById(`speed-${id}`);
        if (speedElement) {
            speedElement.textContent = mediaErrorMessage(error);
            speedElement.title = String(error || 'Process error');
        }
        document.getElementById(`actions-${id}`).innerHTML = `
            <button class="btn-outline" data-action="retry" data-download-id="${id}">Retry</button>
            <button class="btn-icon" data-action="remove" data-download-id="${id}" title="Remove">×</button>
        `;
        updateBadge();
    }

    async function initiateDownload(dataset) {
        if (!currentMetadata) return;

        const downloadId = `dl-${crypto.randomUUID()}`;
        const url = urlInput.value.trim();
        const title = currentMetadata.title || 'Download';
        const thumbnail = safeImageUrl(currentMetadata.thumbnail);

        switchView('view-downloads');

        downloads[downloadId] = {
            state: 'downloading',
            item: { ...dataset },
            url,
            title,
            thumbnail,
            start: null,
            end: null,
        };

        if (
            Number(trimSliderStart.value) > 0 ||
            Number(trimSliderEnd.value) < maxVideoDuration
        ) {
            downloads[downloadId].start = trimInputStart.value;
            downloads[downloadId].end = trimInputEnd.value;
        }

        const html = `
            <div class="dl-item state-downloading" id="${downloadId}">
                <img class="dl-thumb" alt="Thumbnail">
                <div class="dl-info">
                    <div class="dl-title">${escapeHtml(title)}</div>
                    <div class="dl-meta">
                        <span class="dl-status-icon"><svg class="spinner" viewBox="0 0 24 24" style="width:14px;height:14px;"><circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" stroke-dasharray="32" fill="none"></circle></svg></span>
                        <span class="dl-status-text" id="status-text-${downloadId}">Starting…</span>
                        <span>•</span>
                        <span>${escapeHtml(dataset.type.toUpperCase())} / ${escapeHtml(dataset.label)}</span>
                        <span>•</span>
                        <span id="speed-${downloadId}"></span>
                    </div>
                    <div class="dl-progress-container"><div class="dl-progress-bar" id="progress-${downloadId}"></div></div>
                </div>
                <div class="dl-actions" id="actions-${downloadId}">
                    <button class="btn-icon" data-action="cancel" data-download-id="${downloadId}" title="Cancel">
                        <svg viewBox="0 0 24 24"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
                    </button>
                </div>
            </div>`;

        downloadsList.insertAdjacentHTML('afterbegin', html);
        document.getElementById(downloadId).querySelector('.dl-thumb').src = thumbnail;
        updateBadge();

        try {
            await invoke('start_download', {
                id: downloadId,
                url,
                formatId: dataset.id,
                dlType: dataset.type,
                path: appSettings.download_path,
                hasAudio: dataset.hasAudio === 'true',
                sourceExt: dataset.sourceExt || '',
                startTime: downloads[downloadId].start,
                endTime: downloads[downloadId].end,
            });
        } catch (error) {
            markDownloadFailed(downloadId, error);
        }
    }

    await listen('download-progress', (event) => {
        const { id, percent, speed, status, processing } = event.payload;
        const progressElement = document.getElementById(`progress-${id}`);
        const speedElement = document.getElementById(`speed-${id}`);
        const statusElement = document.getElementById(`status-text-${id}`);

        if (progressElement) {
            const visiblePercent = processing ? Math.max(90, Math.min(99, Number(percent))) : Math.min(90, Number(percent) * 0.9);
            progressElement.style.width = `${visiblePercent}%`;
            progressElement.classList.toggle('processing', Boolean(processing));
        }
        if (speedElement) speedElement.textContent = speed || '';
        if (statusElement) statusElement.textContent = status || 'Downloading…';
    });

    await listen('download-completed', (event) => {
        const { id, success, error, file_size } = event.payload;
        if (!downloads[id] || downloads[id].state === 'cancelled') return;

        const element = document.getElementById(id);
        if (!element) return;

        if (!success) {
            markDownloadFailed(id, error);
            return;
        }

        downloads[id].state = 'completed';
        const progressElement = document.getElementById(`progress-${id}`);
        if (progressElement) {
            progressElement.style.width = '100%';
            progressElement.classList.remove('processing');
        }
        element.classList.remove('state-downloading');
        element.classList.add('state-completed');
        element.querySelector('.dl-status-text').textContent = 'Completed';
        element.querySelector('.dl-status-icon').innerHTML = '<svg viewBox="0 0 24 24"><polyline points="20 6 9 17 4 12"></polyline></svg>';
        const finalSize = Number(file_size) > 0
            ? formatBytes(file_size)
            : (downloads[id].start ? 'Size unavailable' : downloads[id].item.size || 'Size unavailable');
        document.getElementById(`speed-${id}`).textContent = finalSize;
        document.getElementById(`actions-${id}`).innerHTML = `
            <button class="btn-outline" data-action="open-folder">Open Folder</button>
            <button class="btn-icon" data-action="remove" data-download-id="${id}" title="Remove">×</button>
        `;
        updateBadge();
    });

    window.cancelJob = async (id) => {
        try {
            await invoke('cancel_download', { id });
        } catch (error) {
            console.warn('Cancel error:', error);
        }

        if (!downloads[id]) return;
        downloads[id].state = 'cancelled';

        const element = document.getElementById(id);
        if (!element) return;

        element.classList.remove('state-downloading');
        element.classList.add('state-failed');
        element.querySelector('.dl-status-text').textContent = 'Cancelled';
        document.getElementById(`speed-${id}`).textContent = '';
        document.getElementById(`actions-${id}`).innerHTML = `
            <button class="btn-outline" data-action="retry" data-download-id="${id}">Retry</button>
            <button class="btn-icon" data-action="remove" data-download-id="${id}" title="Remove">×</button>
        `;
        updateBadge();
    };

    window.removeJob = (id) => {
        document.getElementById(id)?.remove();
        delete downloads[id];
        updateBadge();
    };

    window.retryJob = (id) => {
        const download = downloads[id];
        if (!download) return;

        const retryData = { ...download.item };
        urlInput.value = download.url;
        currentMetadata = {
            ...(currentMetadata || {}),
            title: download.title,
            thumbnail: download.thumbnail,
        };

        window.removeJob(id);
        initiateDownload(retryData);
    };

window.openDir = async () => {
    try {
        await invoke('open_download_folder', {
            path: appSettings.download_path
        });
    } catch (e) {
        alert('Could not open download folder: ' + e);
    }
};

    downloadsList.addEventListener('click', (event) => {
        const button = event.target.closest('button[data-action]');
        if (!button || !downloadsList.contains(button)) return;
        const id = button.dataset.downloadId;
        switch (button.dataset.action) {
            case 'cancel': window.cancelJob(id); break;
            case 'retry': window.retryJob(id); break;
            case 'remove': window.removeJob(id); break;
            case 'open-folder': window.openDir(); break;
        }
    });

    document.getElementById('clear-completed-btn').addEventListener('click', () => {
        Object.keys(downloads).forEach((id) => {
            if (downloads[id].state === 'completed') window.removeJob(id);
        });
    });

    function formatTime(seconds) {
        const value = Math.max(0, Math.floor(Number(seconds) || 0));
        const hours = Math.floor(value / 3600).toString().padStart(2, '0');
        const minutes = Math.floor((value % 3600) / 60).toString().padStart(2, '0');
        const secs = (value % 60).toString().padStart(2, '0');
        return `${hours}:${minutes}:${secs}`;
    }

    function parseTime(value) {
        const parts = String(value).split(':').map(Number);
        if (parts.length !== 3 || parts.some(Number.isNaN)) return 0;
        return parts[0] * 3600 + parts[1] * 60 + parts[2];
    }

    function updateTrimVisuals() {
        if (maxVideoDuration <= 0) {
            trimHighlight.style.left = '0%';
            trimHighlight.style.width = '100%';
            trimInputStart.value = '00:00:00';
            trimInputEnd.value = '00:00:00';
            return;
        }

        let startValue = Number(trimSliderStart.value);
        let endValue = Number(trimSliderEnd.value);

        if (startValue >= endValue) {
            startValue = Math.max(0, endValue - 1);
            trimSliderStart.value = startValue;
        }

        trimHighlight.style.left = `${(startValue / maxVideoDuration) * 100}%`;
        trimHighlight.style.width = `${((endValue - startValue) / maxVideoDuration) * 100}%`;
        trimInputStart.value = formatTime(startValue);
        trimInputEnd.value = formatTime(endValue);
    }

    trimSliderStart.addEventListener('input', updateTrimVisuals);
    trimSliderEnd.addEventListener('input', updateTrimVisuals);

    [trimInputStart, trimInputEnd].forEach((input) => {
        input.addEventListener('change', () => {
            if (maxVideoDuration <= 0) return;

            const isStart = input === trimInputStart;
            let value = Math.max(0, Math.min(parseTime(input.value), maxVideoDuration));

            if (isStart) {
                value = Math.min(value, Number(trimSliderEnd.value) - 1);
                trimSliderStart.value = Math.max(0, value);
            } else {
                value = Math.max(value, Number(trimSliderStart.value) + 1);
                trimSliderEnd.value = Math.min(maxVideoDuration, value);
            }

            updateTrimVisuals();
        });
    });

    navSettingsBtn.addEventListener('click', (event) => {
        event.preventDefault();
        settingsDrawer.classList.add('active');
        syncOverlay();
    });

    closeSettingsBtn.addEventListener('click', () => {
        settingsDrawer.classList.remove('active');
        syncOverlay();
    });

    drawerOverlay.addEventListener('click', () => {
        if (depModal.classList.contains('active') || dependencyOperationRunning) return;
        settingsDrawer.classList.remove('active');
        syncOverlay();
    });

    await initSettings();
});

