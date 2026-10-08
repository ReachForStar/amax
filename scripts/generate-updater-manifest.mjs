// 生成应用内自动更新的版本清单 latest.json。
//
// cargo tauri build 不产出清单（只有 tauri-action 会），所以由这里手工组装，并做两道校验：
// 1. 每个平台的产物各恰好一份（Windows 固定取 zh-CN MSI，Linux 取原始 AppImage——createUpdaterArtifacts: true
//    的 v2 风格下更新包就是原始安装包本身，.tar.gz 只在 v1 兼容模式产出；deb 不进更新通道，客户端提示手动升级）；
// 2. 每个 .sig 与 tauri.conf.json 的 pubkey 是同一对密钥（比对 minisign 文本块里的密钥 id）。
//    tauri build 遇到密钥不匹配只打日志，客户端却会在下载后验签失败，所以这里失败而非告警。
//
// 用法：node scripts/generate-updater-manifest.mjs <版本> <产物目录> <仓库> <标签> <输出路径>
// 产物目录里的文件名需已规范成点形式（见 release.yml 的重命名步骤），
// 否则清单 url 里的空格与 GitHub 资产名的点形式对不上，客户端更新直接 404。
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const [version, distDir, repo, tag, outputPath] = process.argv.slice(2);
if (!version || !distDir || !repo || !tag || !outputPath) {
  console.error('用法：node scripts/generate-updater-manifest.mjs <版本> <产物目录> <仓库> <标签> <输出路径>');
  process.exit(1);
}
if (!existsSync(distDir)) {
  console.error(`::error::产物目录不存在：${distDir}`);
  process.exit(1);
}

const projectRoot = fileURLToPath(new URL('..', import.meta.url));
const conf = JSON.parse(readFileSync(join(projectRoot, 'src-tauri/tauri.conf.json'), 'utf8'));
if (conf.version !== version) {
  console.error(`::error::版本 ${version} 与 src-tauri/tauri.conf.json 的 version=${conf.version} 不一致`);
  process.exit(1);
}

const entries = readdirSync(distDir);

const pickOne = (suffix, label) => {
  const names = entries.filter((name) => name.endsWith(suffix));
  if (names.length !== 1) {
    console.error(`::error::${label} 期望恰好 1 个 *${suffix}，实际 ${names.length} 个（目录内容：${entries.join(', ')}）`);
    process.exit(1);
  }
  const [name] = names;
  if (name.includes(' ')) {
    console.error(`::error::${label} 资产名仍含空格（${name}），清单 url 会与线上资产名不一致`);
    process.exit(1);
  }
  return name;
};

const readSignature = (assetName) => {
  const sigPath = join(distDir, `${assetName}.sig`);
  if (!existsSync(sigPath)) {
    console.error(`::error::缺少更新签名：${assetName}.sig（私钥缺失或无效时构建不签名，发布后自动更新必然失败）`);
    process.exit(1);
  }
  return readFileSync(sigPath, 'utf8').trim();
};

// minisign 文本块第 2 行 base64 的第 3~10 字节是密钥 id
const keyIdOf = (encoded) => {
  const block = Buffer.from(encoded.trim(), 'base64').toString('utf8');
  const line = block.split(/\r?\n/)[1] ?? '';
  const bin = Buffer.from(line, 'base64');
  if (bin.length < 10) {
    throw new Error('不是合法的 minisign 文本块');
  }
  return bin.subarray(2, 10).toString('hex');
};

const winMsi = pickOne(`_${version}_x64_zh-CN.msi`, 'Windows MSI');
const linuxAppImage = pickOne(`_${version}_amd64.AppImage`, 'Linux AppImage 更新包');
const winSig = readSignature(winMsi);
const linuxSig = readSignature(linuxAppImage);

try {
  const pubKeyId = keyIdOf(conf.plugins.updater.pubkey);
  const winKeyId = keyIdOf(winSig);
  const linuxKeyId = keyIdOf(linuxSig);
  if (winKeyId !== pubKeyId || linuxKeyId !== pubKeyId) {
    console.error(`::error::签名密钥 id 不一致（pubkey=${pubKeyId}，Windows=${winKeyId}，Linux=${linuxKeyId}）：私钥与配置里的公钥不是同一对，请成对更新`);
    process.exit(1);
  }
} catch (error) {
  console.error(`::error::pubkey 或 .sig 不是合法的 minisign 文本块：${error.message}`);
  process.exit(1);
}

const baseUrl = `https://github.com/${repo}/releases/download/${tag}`;
const manifest = {
  version,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
  platforms: {
    'windows-x86_64': {
      url: `${baseUrl}/${winMsi}`,
      signature: winSig,
    },
    'linux-x86_64': {
      url: `${baseUrl}/${linuxAppImage}`,
      signature: linuxSig,
    },
  },
};

writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`已生成更新清单：${outputPath}`);
console.log(JSON.stringify(manifest, null, 2));
