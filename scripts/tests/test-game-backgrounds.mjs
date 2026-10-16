import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { readFileSync } from 'node:fs';
import * as fs from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as vue from 'vue';

function load(relative, mocks) {
  const source = readFileSync(new URL(relative, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(compiled, {
    exports, crypto: webcrypto, AbortSignal, console: { ...console, warn() {}, log() {} },
    require(name) {
      assert.ok(name in mocks, `Unexpected import: ${name}`);
      return mocks[name];
    },
  });
  return exports;
}

const pinia = { defineStore: (_id, setup) => { let store; return () => store ??= setup(); } };
const poster = 'https://mc.kurogames.com/website-preface/video/bg/bg-poster.webp';
const video = 'https://mc.kurogames.com/website-preface/video/bg/bg-video.mp4';

async function resources(t, options = {}) {
  const root = await fs.mkdtemp(path.join(tmpdir(), 'ssmt-background-test-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const dir = path.join(root, 'WWMI');
  await fs.mkdir(dir);
  const config = { backgroundType: 'Video', ...options.config };
  const requests = [], signatures = [];
  const manager = load('../../src/store/ResourceManager.ts', {
    pinia,
    './GlobalConfig': { GlobalConfig: { GlobalGamesFolder: async () => root } },
    '../utils/SSMTFileUtils': { SSMTFileUtils: { JoinPath: async (...parts) => path.join(...parts) } },
    '@tauri-apps/plugin-fs': {
      exists: file => fs.access(file).then(() => true, () => false),
      writeFile: fs.writeFile, copyFile: fs.copyFile, remove: fs.rm, mkdir: fs.mkdir,
      rename: async (from, to) => {
        if (options.renameFail && to.endsWith('.mp4')) throw new Error('缓存暂时无法替换');
        await fs.rename(from, to);
      },
      readDir: async dir => (await fs.readdir(dir, { withFileTypes: true }))
        .map(entry => ({ name: entry.name, isDirectory: entry.isDirectory() })),
    },
    '@tauri-apps/api/path': { join: async (...parts) => path.join(...parts) },
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      assert.equal(command, 'sign_wwmi_background_url');
      assert.equal(args.url, video);
      assert.match(args.nonce, /^[0-9a-f]{10}$/);
      signatures.push(args);
      if (options.fail === 'signing') throw new Error('签名不可用');
      return `${args.url}?auth_key=fresh-signature`;
    } },
    '@tauri-apps/plugin-http': { fetch: async url => {
      requests.push(url);
      if (options.posterFail && url === poster) throw new Error('海报下载失败');
      const isVideo = url.startsWith(video);
      if (isVideo) {
        assert.equal(url, `${video}?auth_key=fresh-signature`);
        if (options.fail === 'network') throw new Error('断网');
        if (options.fail === 'timeout') throw new DOMException('超时', 'TimeoutError');
      }
      const bytes = isVideo
        ? Buffer.from(options.fail === 'html' ? '<html>auth expired</html>' : '\0\0\0\x18ftypmp42')
        : Buffer.from('poster fixture');
      return {
        ok: !(isVideo && options.fail === '403'), status: isVideo && options.fail === '403' ? 403 : 200,
        arrayBuffer: async () => bytes,
      };
    } },
    '../utils/SSMTJsonUtils': {}, '../helper/PathHelper': {},
    '../i18n': { i18n: { global: { t: key => key } } }, '../utils/debugLog': {},
    './GameConfig': { GameConfigManager: {
      loadGameConfig: async () => structuredClone(config),
      saveGameConfig: async (_name, updated) => Object.assign(config, structuredClone(updated)),
    } },
    './GamePreset': {},
  }).ResourceManager;
  const update = type => manager.updateGameBackground('WWMI', 'WWMI', type || 'Video', config.lastBackgroundUrl);
  return { config, requests, signatures, dir, update };
}

test('鸣潮动态和静态各占一个列表项，共用海报且不保存临时签名', async t => {
  const h = await resources(t);
  await h.update();
  assert.deepEqual(h.config.backgroundEntries.map(entry => entry.type), ['Image', 'Video']);
  const [image, motion] = h.config.backgroundEntries;
  assert.equal(motion.imagePath, image.imagePath);
  assert.equal(motion.videoUrl, video);
  assert.equal(h.config.selectedBackgroundId, motion.id);
  assert.equal(h.requests.length, 2);
  assert.equal(h.signatures.length, 1);
  assert.ok(!JSON.stringify(h.config).includes('auth_key'));
  assert.equal((await fs.readFile(motion.videoPath)).subarray(4, 8).toString(), 'ftyp');
  assert.ok(!(await fs.readdir(h.dir)).some(name => name.endsWith('.download')));
  h.requests.length = 0;
  h.signatures.length = 0;
  const unchanged = await h.update();
  assert.equal(unchanged.changed, false);
  assert.equal(h.requests.length, 0);
  assert.equal(h.signatures.length, 0);
  await h.update('Image');
  assert.equal(h.config.selectedBackgroundId, image.id);
});

for (const fail of ['signing', '403', 'network', 'timeout', 'html']) {
  test(`动态背景 ${fail} 时回退静态，不留下无效视频列表项`, async t => {
    const h = await resources(t, { fail });
    const result = await h.update();
    assert.equal(h.config.backgroundEntries.length, 1);
    assert.equal(h.config.backgroundEntries[0].type, 'Image');
    assert.equal(h.config.selectedBackgroundId, h.config.backgroundEntries[0].id);
    assert.equal(result.path, h.config.backgroundEntries[0].imagePath);
    assert.equal(await fs.readFile(result.path, 'utf8'), 'poster fixture');
    assert.equal(h.config.backgroundType, 'Video', '临时失败仍保留用户对动态背景的偏好');
    assert.ok(!(await fs.readdir(h.dir)).some(name => /\.mp4|\.download$/.test(name)));
  });
}

test('海报和视频均不可获取时使用已有本地静态背景', async t => {
  const h = await resources(t, { fail: '403', posterFail: true });
  const cached = path.join(h.dir, 'Background.webp');
  await fs.writeFile(cached, 'existing static background');
  const result = await h.update();
  assert.equal(result.path, cached);
  assert.equal(h.config.backgroundEntries.length, 1);
  assert.equal(h.requests.filter(url => url === poster).length, 1);
  assert.equal(await fs.readFile(cached, 'utf8'), 'existing static background');
  assert.equal(h.config.backgroundEntries[0].imageUrl, undefined, '本地回退不冒充新资源，联网后仍应重试');
});

test('完全离线且没有缓存时保留原配置，由界面使用内置静态背景', async t => {
  const h = await resources(t, { fail: 'network', posterFail: true });
  const result = await h.update();
  assert.equal(result.changed, false);
  assert.equal(h.config.backgroundEntries, undefined);
  assert.deepEqual(await fs.readdir(h.dir), []);
});

test('替换视频失败时保留原缓存并允许下次重试', async t => {
  const options = {};
  const h = await resources(t, options);
  await h.update();
  const cached = h.config.backgroundEntries[1].videoPath;
  await fs.writeFile(cached, 'previous working video');
  h.config.backgroundEntries[1].videoUrl = 'https://mc.kurogames.com/old-video.mp4';
  options.renameFail = true;
  await h.update();
  assert.equal(await fs.readFile(cached, 'utf8'), 'previous working video');
  assert.equal(h.config.backgroundEntries[1].videoUrl, 'https://mc.kurogames.com/old-video.mp4');
  assert.ok(!(await fs.readdir(h.dir)).some(name => name.endsWith('.download')));
  options.renameFail = false;
  await h.update();
  assert.equal(h.config.backgroundEntries[1].videoUrl, video);
  assert.equal((await fs.readFile(cached)).subarray(4, 8).toString(), 'ftyp');
});

function state(t) {
  const scope = vue.effectScope();
  t.after(() => scope.stop());
  return scope.run(() => load('../../src/store/AppStateManager.ts', {
    vue, pinia, 'element-plus': {}, '@tauri-apps/api/core': {}, '@tauri-apps/api/window': {},
    '@tauri-apps/api/app': {}, './AppTheme': {}, './GlobalConfig': {},
    './GamePreset': { AUTO_UPDATE_SUPPORTED_PRESET_SET: new Set() }, './ResourceManager': {},
    './AppSettings': {
      BGType: { Image: 'Image', Video: 'Video' },
      AppSettings: class { bgType = 'Image'; bgImage = ''; bgVideo = ''; CurrentGameName = ''; },
    },
    '../i18n': { i18n: { global: { t: key => key } } },
  }).AppStateManager);
}

function game(name = 'WWMI') {
  return {
    name, bgType: 'Video',
    backgroundEntries: [
      { id: 'image', type: 'Image', imagePath: `asset://${name}/poster.webp` },
      { id: 'video', type: 'Video', videoPath: `asset://${name}/video.mp4`, themePath: 'asset://theme.webp' },
    ],
  };
}

test('解码或播放错误回退海报，保持所选背景以便下次重试', t => {
  const h = state(t), current = game();
  h.applyBackgroundEntry(current, 1);
  assert.equal(h.appSettings.bgImage, 'asset://WWMI/poster.webp');
  h.fallbackBackgroundVideo(h.appSettings.bgVideo);
  assert.equal(h.appSettings.bgType, 'Image');
  assert.equal(h.appSettings.bgImage, 'asset://WWMI/poster.webp');
  assert.equal(h.appSettings.bgVideo, '');
  assert.equal(h.appSettings.bgTheme, '');
  assert.equal(current.bgIndex, 1);
  h.applyBackgroundEntry(current, 1);
  assert.equal(h.appSettings.bgType, 'Video');
});

test('旧视频延迟报错不会覆盖新游戏或已切换的静态图', t => {
  const h = state(t);
  h.applyBackgroundEntry(game(), 1);
  const old = h.appSettings.bgVideo;
  h.applyBackgroundEntry(game('GIMI'), 1);
  h.fallbackBackgroundVideo(old);
  assert.equal(h.appSettings.bgType, 'Video');
  assert.equal(h.appSettings.bgVideo, 'asset://GIMI/video.mp4');
  h.applyBackgroundEntry(game('GIMI'), 0);
  h.fallbackBackgroundVideo('asset://GIMI/video.mp4');
  assert.equal(h.appSettings.bgImage, 'asset://GIMI/poster.webp');
});

test('没有可用海报时播放失败使用内置静态背景', t => {
  const h = state(t), current = game();
  current.backgroundEntries.shift();
  h.applyBackgroundEntry(current, 0);
  h.fallbackBackgroundVideo(h.appSettings.bgVideo);
  assert.equal(h.appSettings.bgType, 'Image');
  assert.equal(h.appSettings.bgImage, '/Background.png');
});
