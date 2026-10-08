// 提取 CHANGELOG.md 中指定版本的 Release Notes；缺小节时回退为提交列表（既定行为）。
//
// 用法：
//   node scripts/release-notes.mjs <版本>           输出 Notes 到 stdout（发布用）
//   node scripts/release-notes.mjs <版本> --check   校验标签版本与配置版本一致；
//                                                   CHANGELOG 缺小节只告警，不算失败
// 回退逻辑用 git describe 找上一个标签（发布作业需 fetch-depth: 0 才能取到历史与标签）。
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = fileURLToPath(new URL('..', import.meta.url));
const [version, flag] = process.argv.slice(2);
if (!version) {
  console.error('用法：node scripts/release-notes.mjs <版本> [--check]');
  process.exit(1);
}

const extractSection = (targetVersion) => {
  const lines = readFileSync(join(projectRoot, 'CHANGELOG.md'), 'utf8').split(/\r?\n/);
  const start = lines.findIndex((line) => line.startsWith(`## [${targetVersion}]`));
  if (start === -1) {
    return null;
  }
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i += 1) {
    if (lines[i].startsWith('## [')) {
      end = i;
      break;
    }
  }
  return lines.slice(start + 1, end).join('\n').trim() || null;
};

if (flag === '--check') {
  const conf = JSON.parse(readFileSync(join(projectRoot, 'src-tauri/tauri.conf.json'), 'utf8'));
  if (conf.version !== version) {
    console.error(`::error::标签 v${version} 与 src-tauri/tauri.conf.json 的 version=${conf.version} 不一致`);
    process.exit(1);
  }
  if (!extractSection(version)) {
    console.warn(`::warning::CHANGELOG.md 缺少 [${version}] 小节，Release Notes 将回退为提交列表`);
  }
  console.log(`标签版本与配置版本一致：v${version}`);
  process.exit(0);
}

const section = extractSection(version);
if (section) {
  process.stdout.write(`${section}\n`);
  process.exit(0);
}

const tag = `v${version}`;
let previous = '';
try {
  previous = execFileSync('git', ['describe', '--tags', '--abbrev=0', `${tag}^`], { cwd: projectRoot, encoding: 'utf8' }).trim();
} catch {
  previous = '';
}
const range = previous ? `${previous}..${tag}` : tag;
let commits = '';
try {
  commits = execFileSync('git', ['log', range, '--pretty=format:- %s'], { cwd: projectRoot, encoding: 'utf8' }).trim();
} catch {
  console.error(`::error::无法从 git 历史生成提交列表（标签 ${tag} 不存在，或 checkout 未取到历史）`);
  process.exit(1);
}
process.stdout.write(`## 修改说明\n\n${commits}\n`);
