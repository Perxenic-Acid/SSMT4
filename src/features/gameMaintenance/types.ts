export interface GameMaintenanceStatus {
  servers: Array<{ id: string; name: string }>;
  currentServer: string | null;
  selectedServer: string;
  installedVersion: string | null;
  clientFound: boolean;
  latestVersion: string;
  preDownloadVersion: string | null;
  directory: string | null;
  targetExe: string;
  audioLanguages: string[];
  recoveryPending: boolean;
}
export interface GameMaintenanceTask {
  id: string;
  gameName: string;
  directory: string;
  operation: string;
  targetVersion: string;
  state: string;
  phase: string;
  message: string;
  bytesDone: number;
  bytesTotal: number;
  filesDone: number;
  filesTotal: number;
}
