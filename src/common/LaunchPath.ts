import { stat } from '@tauri-apps/plugin-fs';

// 用户可以指定未知名称的程序，但目录、失效链接和不可访问的路径都不能当成程序。
export async function isExistingLaunchFile(path: string, timeoutMs = 1500): Promise<boolean> {
  const value = (path || '').trim();
  if (!value) return false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      stat(value).then(info => info.isFile),
      new Promise<boolean>(resolve => { timer = setTimeout(() => resolve(false), timeoutMs); }),
    ]);
  } catch { return false; }
  finally { if (timer) clearTimeout(timer); }
}
