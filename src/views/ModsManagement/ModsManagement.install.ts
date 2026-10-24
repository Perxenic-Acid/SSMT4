import { reactive, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ElMessage } from 'element-plus';
import { ModManager } from '../../store/ModManager';
import { ModTagStore } from '../../store/ModTagStore';
import type { ArchivePreview, InstallFormState, InstallProgressState } from './ModsManagement.types';
import type { Ref } from 'vue';
import { debugError } from '../../utils/debugLog';

type Translate = (key: string, params?: Record<string, unknown>) => string;

interface UseModsManagementInstallOptions {
    selectedGame: Ref<string>;
    selectedGroup: Ref<string>;
    pendingDropGroup?: Ref<string | null>;
    loading: Ref<boolean>;
    t: Translate;
    reconcileInstalledMods: () => Promise<void>;
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

    const handleFileDrop = async (path: string) => {
        const lower = path.toLowerCase();
        const isArchive = lower.endsWith('.zip') || lower.endsWith('.7z') || lower.endsWith('.rar');
        const maybeFolder = !isArchive;

        if (!isArchive && !maybeFolder) {
            return;
        }

        installForm.archivePath = path;

        const filename = path.split(/[\\/]/).pop() || 'New Mod';
        installForm.modName = filename.replace(/\.(zip|7z|rar)/i, '');
        
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
            installPreview.value = await invoke<ArchivePreview>('preview_mod_archive', { path });
            showInstallDialog.value = true;
        } catch (error) {
            ElMessage.error({
                message: options.t('modsManagement.messages.dragDropSupportedOnly', { error: String(error) }),
                offset: 48,
            });
        } finally {
            options.loading.value = false;
        }
    };

    const pickInstallArchive = async () => {
        try {
            const selected = await open({
                multiple: false,
                filters: [{
                    name: 'Archives',
                    extensions: ['zip', '7z', 'rar'],
                }],
            });

            if (selected && typeof selected === 'string') {
                await handleFileDrop(selected);
            }
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
                multiple: false,
            });

            if (selected && typeof selected === 'string') {
                await handleFileDrop(selected);
            }
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
    };

    const confirmInstall = async () => {
        if (!installForm.modName) {
            ElMessage.warning({ message: options.t('modsManagement.messages.enterModName'), offset: 48 });
            return;
        }

        isInstalling.value = true;
        installProgress.visible = true;
        installProgress.percent = 0;
        installProgress.stage = options.t('modsManagement.progress.preparing');
        try {
            options.suppressFsRefresh?.(30_000);
            const installDir = await ModManager.getInstallDir(options.selectedGame.value);
            await invoke('install_mod_archive', {
                gameName: options.selectedGame.value,
                installDir,
                archivePath: installForm.archivePath,
                targetName: installForm.modName,
                targetGroup: installForm.targetGroup,
                password: installForm.password || null,
            });
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
                    await ModTagStore.setModTags(options.selectedGame.value, modRelativePath, installForm.selectedTagIds);
                } catch (tagErr) {
                    console.warn('Failed to set tags for newly installed mod:', tagErr);
                }
            }

            await options.reconcileInstalledMods();

            ElMessage.success({ message: options.t('modsManagement.messages.installedSuccessfully'), offset: 48 });
            setTimeout(() => {
                showInstallDialog.value = false;
            }, 400);
        } catch (error) {
            ElMessage.error({ message: options.t('modsManagement.messages.installFailed', { error: String(error) }), offset: 48 });
            installProgress.stage = options.t('modsManagement.progress.installFailed');
        } finally {
            isInstalling.value = false;
            setTimeout(() => {
                installProgress.visible = false;
            }, 800);
        }
    };

    const batchInstallFromPaths = async (paths: string[]): Promise<void> => {
        // Filter to archive files only
        const installablePaths: string[] = [];

        for (const path of paths) {
            const lower = path.toLowerCase();
            if (lower.endsWith('.zip') || lower.endsWith('.7z') || lower.endsWith('.rar')) {
                installablePaths.push(path);
            }
        }

        if (installablePaths.length === 0) {
            ElMessage.warning({
                message: options.t('modsManagement.messages.dragDropSupportedOnly', { error: '' }),
                offset: 48,
            });
            return;
        }

        // Show progress
        options.loading.value = true;
        installProgress.visible = true;
        installProgress.percent = 0;
        installProgress.stage = options.t('modsManagement.progress.preparing');
        options.suppressFsRefresh?.(30_000);

        let successCount = 0;
        let failCount = 0;

        for (let i = 0; i < installablePaths.length; i++) {
            const path = installablePaths[i];
            const filename = path.split(/[\\/]/).pop() || 'New Mod';
            const modName = filename.replace(/\.(zip|7z|rar)/i, '');
            const dropTarget = options.pendingDropGroup?.value ?? null;
            let targetGroup: string;
            if (dropTarget) {
                targetGroup = dropTarget === 'Root' || dropTarget === 'All' ? 'Default' : dropTarget;
                options.pendingDropGroup!.value = null;
            } else {
                targetGroup = options.selectedGroup.value === 'All' ? 'Default' : options.selectedGroup.value;
            }

            installProgress.stage = `${filename} (${i + 1}/${installablePaths.length})`;
            installProgress.percent = Math.round((i / installablePaths.length) * 100);

            try {
                // Validate archive
                await invoke<ArchivePreview>('preview_mod_archive', { path });

                // Install silently
                options.suppressFsRefresh?.(30_000);
                const installDir = await ModManager.getInstallDir(options.selectedGame.value);
                await invoke('install_mod_archive', {
                    gameName: options.selectedGame.value,
                    installDir,
                    archivePath: path,
                    targetName: modName,
                    targetGroup,
                    password: null,
                });

                successCount++;
            } catch (error) {
                debugError('BatchInstall', `Failed to install ${filename}:`, error);
                failCount++;
            }
        }

        installProgress.percent = 100;
        installProgress.stage = options.t('modsManagement.progress.extractionComplete');
        options.suppressFsRefresh?.(2_500);
        options.loading.value = false;

        if (successCount > 0) {
            await options.reconcileInstalledMods();
            ElMessage.success({
                message: options.t('modsManagement.messages.batchInstallComplete', { success: successCount, fail: failCount }),
                offset: 48,
            });
        }

        setTimeout(() => {
            installProgress.visible = false;
        }, 1500);
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
