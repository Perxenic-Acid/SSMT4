import { h, reactive, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ElMessage, ElMessageBox } from 'element-plus';
import { ModManager } from '../../store/ModManager';
import { ModTagStore } from '../../store/ModTagStore';
import type { ArchivePreview, InstallFormState, InstallProgressState } from './ModsManagement.types';
import type { Ref } from 'vue';
import { debugError } from '../../utils/debugLog';
import './ModsManagement.import-dialog.css';

type Translate = (key: string, params?: Record<string, unknown>) => string;

interface UseModsManagementInstallOptions {
    selectedGame: Ref<string>;
    selectedGroup: Ref<string>;
    pendingDropGroup?: Ref<string | null>;
    loading: Ref<boolean>;
    t: Translate;
    reconcileInstalledMods: (gameName: string, targetGroup: string, names: string[]) => Promise<void>;
    suppressFsRefresh?: (durationMs?: number) => void;
}

export const useModsManagementInstall = (options: UseModsManagementInstallOptions) => {
    const showInstallDialog = ref(false);
    const installPreview = ref<ArchivePreview | null>(null);
    const isInstalling = ref(false);
    const installForm = reactive<InstallFormState>({
        archivePath: '',
        modName: '',
        targetGroup: '',
        password: '',
        selectedTagIds: [],
    });
    const installProgress = reactive<InstallProgressState>({
        visible: false,
        percent: 0,
        stage: 'Preparing',
    });

    let importBusy = false;
    let progressGeneration = 0;
    const requestPassword = async (path: string, incorrect: boolean): Promise<string | null> => {
        const filename = path.split(/[\\/]/).pop() || path;
        try {
            const result = await ElMessageBox.prompt(
                options.t(incorrect ? 'modsManagement.messages.archivePasswordIncorrect' : 'modsManagement.messages.archivePasswordRequired', { filename }),
                options.t('modsManagement.messages.archivePasswordTitle'),
                {
                    inputType: 'password', inputValue: '', closeOnClickModal: false,
                    confirmButtonText: options.t('modsManagement.messages.archivePasswordConfirm'),
                    cancelButtonText: options.t('modsManagement.messages.archivePasswordSkip'),
                    inputValidator: (value: string) => value.length > 0 || options.t('modsManagement.messages.archivePasswordEmpty'),
                },
            );
            return result.value;
        } catch { return null; }
    };
    const withPassword = async <T>(path: string, credential: { password: string | null }, operation: (password: string | null) => Promise<T>): Promise<T> => {
        const previousPassword = credential.password;
        while (true) {
            try { return await operation(credential.password); }
            catch (error) {
                const code = String(error);
                if (!/^ARCHIVE_PASSWORD_(REQUIRED|INVALID)(?:$|:)/.test(code)) throw error;
                const password = await requestPassword(path, code.startsWith('ARCHIVE_PASSWORD_INVALID') || credential.password !== null);
                if (password === null) { credential.password = previousPassword; throw passwordCancelled; }
                credential.password = password;
            }
        }
    };
    const installWithConflicts = async (args: Record<string, unknown>, credential: { password: string | null }): Promise<string> => {
        let name = String(args.targetName);
        let conflictAction = 'ask';
        while (true) {
            try {
                await withPassword(String(args.archivePath), credential, password => invoke('install_mod_archive', { ...args, targetName: name, conflictAction, password }));
                return name;
            } catch (error) {
                if (!String(error).startsWith('MOD_IMPORT_CONFLICT')) throw error;
                const choice = ref('skip');
                try {
                    await ElMessageBox({
                        title: options.t('modsManagement.messages.importConflictTitle'),
                        customClass: 'mod-import-conflict',
                        message: () => h('div', { class: 'mod-import-conflict__body' }, [
                            h('p', { class: 'mod-import-conflict__intro' }, options.t('modsManagement.messages.importConflictPrompt')),
                            h('div', { class: 'mod-import-conflict__name', title: name }, name),
                            h('div', { class: 'mod-import-conflict__options', role: 'radiogroup', 'aria-label': options.t('modsManagement.messages.importConflictTitle') },
                                ['skip', 'replace', 'rename', 'merge'].map(value => h('label', {
                                    class: ['mod-import-conflict__option', { 'is-selected': choice.value === value }],
                                }, [
                                    h('input', { type: 'radio', name: 'mod-import-conflict-action', value, checked: choice.value === value, onChange: () => { choice.value = value; } }),
                                    h('span', { class: 'mod-import-conflict__copy' }, [
                                        h('span', { class: 'mod-import-conflict__label' }, options.t(`modsManagement.messages.importConflict_${value}`)),
                                        h('span', { class: 'mod-import-conflict__hint' }, options.t(`modsManagement.messages.importConflictHint_${value}`)),
                                    ]),
                                ]))),
                        ]),
                        showCancelButton: true, closeOnClickModal: false,
                        confirmButtonText: options.t('modsManagement.messages.archivePasswordConfirm'),
                        cancelButtonText: options.t('modsManagement.messages.archivePasswordSkip'),
                    });
                } catch { throw passwordCancelled; }
                if (choice.value === 'skip') throw passwordCancelled;
                if (choice.value === 'rename') {
                    try {
                        const result = await ElMessageBox.prompt(options.t('modsManagement.messages.importConflict_rename'), options.t('modsManagement.messages.importConflictTitle'), {
                            inputValue: `${name} (2)`, closeOnClickModal: false,
                            inputValidator: (value: string) => !!value.trim() || options.t('modsManagement.messages.enterModName'),
                        });
                        name = result.value.trim();
                        conflictAction = 'ask';
                    } catch { throw passwordCancelled; }
                } else conflictAction = choice.value;
            }
        }
    };
    const passwordCancelled = Symbol('passwordCancelled');
    const handleFileDrop = async (path: string) => {
        if (importBusy || isInstalling.value || showInstallDialog.value) return;
        importBusy = true;
        ++progressGeneration;
        installProgress.visible = false;
        installForm.archivePath = path;

        const filename = path.split(/[\\/]/).pop() || 'New Mod';
        installForm.modName = filename.replace(/\.(zip|7z|rar)$/i, '');
        
        // If a file was dropped on a specific group tree node, use that group; otherwise fall back to selectedGroup
        const dropTarget = options.pendingDropGroup?.value ?? null;
        if (dropTarget) {
            installForm.targetGroup = dropTarget === 'Root' || dropTarget === 'All' ? 'Default' : dropTarget;
            options.pendingDropGroup!.value = null;
        } else {
            installForm.targetGroup = options.selectedGroup.value === 'All' ? 'Default' : options.selectedGroup.value;
        }
        installForm.password = '';
        installForm.selectedTagIds = [];

        try {
            options.loading.value = true;
            const credential = { password: null as string | null };
            installPreview.value = await withPassword(path, credential, password => invoke<ArchivePreview>('preview_mod_archive', { path, password }));
            installForm.password = credential.password ?? '';
            showInstallDialog.value = true;
        } catch (error) {
            if (error === passwordCancelled) return;
            ElMessage.error({
                message: options.t('modsManagement.messages.dragDropSupportedOnly', { error: String(error) }),
                offset: 48,
            });
        } finally {
            options.loading.value = false;
            importBusy = false;
        }
    };

    const pickInstallArchive = async () => {
        try {
            const selected = await open({
                multiple: true,
                filters: [{
                    name: 'Archives',
                    extensions: ['zip', '7z', 'rar'],
                }],
            });

            const paths = typeof selected === 'string' ? [selected] : selected ?? [];
            if (paths.length === 1) await handleFileDrop(paths[0]);
            else if (paths.length > 1) await batchInstallFromPaths(paths);
        } catch (error) {
            ElMessage.error({
                message: options.t('modsManagement.messages.selectInstallSourceFailed', { error: String(error) }),
                offset: 48,
            });
        }
    };

    const pickInstallFolder = async () => {
        try {
            const selected = await open({
                directory: true,
                multiple: true,
            });

            const paths = typeof selected === 'string' ? [selected] : selected ?? [];
            if (paths.length === 1) await handleFileDrop(paths[0]);
            else if (paths.length > 1) await batchInstallFromPaths(paths);
        } catch (error) {
            ElMessage.error({
                message: options.t('modsManagement.messages.selectInstallSourceFailed', { error: String(error) }),
                offset: 48,
            });
        }
    };

    const cancelInstall = () => {
        if (isInstalling.value) return;
        showInstallDialog.value = false;
        installForm.password = '';
    };

    const confirmInstall = async () => {
        if (!installForm.modName) {
            ElMessage.warning({ message: options.t('modsManagement.messages.enterModName'), offset: 48 });
            return;
        }

        const generation = ++progressGeneration;
        isInstalling.value = true;
        installProgress.visible = true;
        installProgress.percent = 0;
        installProgress.stage = options.t('modsManagement.progress.preparing');
        try {
            options.suppressFsRefresh?.(30_000);
            const gameName = options.selectedGame.value;
            const installDir = await ModManager.getInstallDir(gameName);
            const credential = { password: installForm.password || null };
            installForm.modName = await installWithConflicts({
                gameName,
                installDir,
                archivePath: installForm.archivePath,
                targetName: installForm.modName,
                targetGroup: installForm.targetGroup,
            }, credential);
            installForm.password = credential.password ?? '';
            options.suppressFsRefresh?.(2_500);
            installProgress.stage = options.t('modsManagement.progress.extractionComplete');
            installProgress.percent = 100;

            // Apply selected tags to the newly installed mod.
            if (installForm.selectedTagIds.length > 0) {
                const targetGroup = installForm.targetGroup.trim();
                const modRelativePath = targetGroup && targetGroup !== 'Root'
                    ? `${targetGroup}/${installForm.modName}`
                    : installForm.modName;
                try {
                    await ModTagStore.setModTags(gameName, modRelativePath, installForm.selectedTagIds);
                } catch (tagErr) {
                    console.warn('Failed to set tags for newly installed mod:', tagErr);
                }
            }

            await options.reconcileInstalledMods(gameName, installForm.targetGroup, [installForm.modName]);

            ElMessage.success({ message: options.t('modsManagement.messages.installedSuccessfully'), offset: 48 });
            setTimeout(() => {
                if (generation !== progressGeneration) return;
                showInstallDialog.value = false;
                installForm.password = '';
            }, 400);
        } catch (error) {
            if (error === passwordCancelled) { showInstallDialog.value = false; installForm.password = ''; return; }
            ElMessage.error({ message: options.t('modsManagement.messages.installFailed', { error: String(error) }), offset: 48 });
            installProgress.stage = options.t('modsManagement.progress.installFailed');
        } finally {
            isInstalling.value = false;
            setTimeout(() => { if (generation === progressGeneration) installProgress.visible = false; }, 800);
        }
    };

    const batchInstallFromPaths = async (paths: string[]): Promise<void> => {
        if (importBusy || isInstalling.value || showInstallDialog.value) return;
        const installablePaths = [...new Set(paths)];
        if (!installablePaths.length) return;
        importBusy = true;
        isInstalling.value = true;
        const generation = ++progressGeneration;
        const gameName = options.selectedGame.value;
        const dropTarget = options.pendingDropGroup?.value;
        if (options.pendingDropGroup) options.pendingDropGroup.value = null;
        const group = options.selectedGroup.value;
        const targetGroup = dropTarget
            ? (dropTarget === 'Root' || dropTarget === 'All' ? 'Default' : dropTarget)
            : (group === 'All' ? 'Default' : group);
        const credential = { password: null as string | null };

        // Show progress
        installProgress.visible = true;
        installProgress.percent = 0;
        installProgress.stage = options.t('modsManagement.progress.preparing');
        options.suppressFsRefresh?.(30_000);

        let successCount = 0;
        let failCount = 0;
        let skippedCount = 0;
        const installedNames: string[] = [];
        const failures: string[] = [];

        try {
            const installDir = await ModManager.getInstallDir(gameName);
            for (let i = 0; i < installablePaths.length; i++) {
                const path = installablePaths[i];
                const filename = path.split(/[\\/]/).pop() || 'New Mod';
                const modName = filename.replace(/\.(zip|7z|rar)$/i, '');
                installProgress.stage = `${filename} (${i + 1}/${installablePaths.length})`;
                installProgress.percent = Math.round((i / installablePaths.length) * 100);

                try {
                    // Validate archive
                    await withPassword(path, credential, password => invoke<ArchivePreview>('preview_mod_archive', { path, password }));

                    // Install silently
                    options.suppressFsRefresh?.(30_000);
                    const installedName = await installWithConflicts({
                        gameName,
                        installDir,
                        archivePath: path,
                        targetName: modName,
                        targetGroup,
                    }, credential);

                    successCount++;
                    installedNames.push(installedName);
                } catch (error) {
                    if (error === passwordCancelled) skippedCount++;
                    else {
                        debugError('BatchInstall', `Failed to install ${filename}:`, error);
                        failures.push(`${filename}: ${String(error)}`);
                        failCount++;
                    }
                }
            }

            installProgress.percent = 100;
            installProgress.stage = options.t('modsManagement.progress.extractionComplete');
            options.suppressFsRefresh?.(2_500);

            if (successCount > 0) {
                try {
                    await options.reconcileInstalledMods(gameName, targetGroup, installedNames);
                } catch (error) {
                    ElMessage.error({ message: options.t('modsManagement.messages.loadModFailed', { error: String(error) }), offset: 48 });
                }
            }
            ElMessage[failCount || skippedCount ? 'warning' : 'success']({
                message: options.t('modsManagement.messages.batchImportSummary', { success: successCount, fail: failCount, skipped: skippedCount }),
                offset: 48,
            });
            if (failures.length) {
                await ElMessageBox.alert(failures.join("\n\n"), options.t('modsManagement.progress.installFailed'), {
                    customStyle: { whiteSpace: 'pre-wrap' },
                    dangerouslyUseHTMLString: false,
                }).catch(() => {});
            }

        } finally {
            credential.password = null;
            isInstalling.value = false;
            importBusy = false;
            options.suppressFsRefresh?.(2_500);
            setTimeout(() => { if (generation === progressGeneration) installProgress.visible = false; }, 1500);
        }
    };

    const fetchGroupSuggestions = (groupIds: string[]) => (query: string, callback: (items: Array<{ value: string }>) => void) => {
        callback(
            groupIds
                .filter((id) => id !== 'All' && id.toLowerCase().includes(query.toLowerCase()))
                .map((value) => ({ value })),
        );
    };

    return {
        showInstallDialog,
        installForm,
        installPreview,
        isInstalling,
        installProgress,
        handleFileDrop,
        batchInstallFromPaths,
        pickInstallArchive,
        pickInstallFolder,
        cancelInstall,
        confirmInstall,
        fetchGroupSuggestions,
    };
};
