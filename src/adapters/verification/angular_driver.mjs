// Closed driver: configuration and compilation are handled by the project's
// own TypeScript and @angular/compiler-cli. Never install packages or emit files.
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {pathToFileURL} from 'node:url';

const request = JSON.parse(process.argv[1]);
const root = fs.realpathSync(process.cwd());
const result = {
  harnessAngularVerification: 1, compiler: '@angular/compiler-cli',
  verificationMode: 'angular_official_compiler', verificationAvailable: false,
  typeCheckPerformed: false, templateTypeCheckPerformed: false,
  noEmit: true, bundleBuildPerformed: false, diagnostics: [],
};
function fail(code, message) {
  result.diagnostics.push({code, message, severity: 'error'});
  throw new Error('harness-angular-reported');
}
function inside(file) {
  const relative = path.relative(root, file);
  return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}
function localFile(file) {
  const resolved = fs.realpathSync(file);
  if (!inside(resolved)) fail('PATH', 'Angular configuration must be inside the registered project');
  return resolved;
}
function relative(file) { return path.relative(root, file).split(path.sep).join('/'); }
try {
  const scope = localFile(path.resolve(root, request.scope));
  let directory = fs.statSync(scope).isDirectory() ? scope : path.dirname(scope);
  let workspace = null;
  const explicitConfig = !fs.statSync(scope).isDirectory() && path.basename(scope) !== 'angular.json' && scope.endsWith('.json');
  if (!explicitConfig) {
    while (inside(directory)) {
      const candidate = path.join(directory, 'angular.json');
      if (fs.existsSync(candidate)) { workspace = localFile(candidate); break; }
      if (directory === root) break;
      directory = path.dirname(directory);
    }
    if (!workspace) fail('ANGULAR_WORKSPACE', 'No angular.json found inside the registered root; scope may also select an explicit tsconfig JSON file');
  }
  const angularRoot = workspace ? path.dirname(workspace) : path.dirname(scope);
  const require = createRequire(path.join(angularRoot, 'package.json'));
  let compilerEntry, ts;
  try {
    compilerEntry = require.resolve('@angular/compiler-cli');
    const compilerRequire = createRequire(compilerEntry);
    ts = compilerRequire('typescript');
  } catch (error) {
    fail('ANGULAR_DEPENDENCIES', `Install the project's compatible @angular/compiler-cli, @angular/compiler and typescript dependencies first: ${error.message}`);
  }
  const compiler = await import(pathToFileURL(compilerEntry).href);
  const packagePath = require.resolve('@angular/compiler-cli/package.json');
  const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
  result.compilerVersion = packageJson.version;
  result.verificationAvailable = true;
  function diagnostic(d) {
    const item = {
      code: d.code < -990000 ? `NG${-d.code - 990000}` : `TS${d.code}`,
      message: ts.flattenDiagnosticMessageText(d.messageText, '\n'),
      severity: d.category === ts.DiagnosticCategory.Error ? 'error' : 'warning',
    };
    if (d.file && typeof d.start === 'number') {
      const position = d.file.getLineAndCharacterOfPosition(d.start);
      item.path = inside(d.file.fileName) ? relative(d.file.fileName) : d.file.fileName;
      item.line = position.line + 1;
      item.column = position.character + 1;
    }
    return item;
  }
  let configPath = scope;
  if (workspace) {
    const parsed = ts.parseConfigFileTextToJson(workspace, fs.readFileSync(workspace, 'utf8'));
    if (parsed.error) { result.diagnostics.push(diagnostic(parsed.error)); throw new Error('harness-angular-reported'); }
    const projects = parsed.config.projects || {};
    const buildFor = project => project?.architect?.build ?? project?.targets?.build;
    const candidates = Object.keys(projects).filter(name => buildFor(projects[name]));
    const name = request.project ?? (candidates.length === 1 ? candidates[0] : null);
    if (!name || !buildFor(projects[name])) fail('ANGULAR_PROJECT', `Select a project with a build target using project; candidates: ${candidates.join(', ') || '(none)'}`);
    const build = buildFor(projects[name]);
    let options = {...build.options};
    const configuration = request.configuration ?? build.defaultConfiguration;
    if (configuration) {
      for (const selected of configuration.split(',')) {
        if (!Object.hasOwn(build.configurations || {}, selected)) fail('ANGULAR_CONFIGURATION', `Unknown Angular build configuration: ${selected}`);
        options = {...options, ...build.configurations[selected]};
      }
    }
    if (typeof options.tsConfig !== 'string') fail('ANGULAR_TSCONFIG', 'The selected Angular build target does not specify tsConfig; select its tsconfig JSON file with scope');
    configPath = localFile(path.resolve(angularRoot, options.tsConfig));
    result.project = name;
    result.configuration = configuration ?? null;
  } else if (request.project || request.configuration) {
    fail('ANGULAR_OPTIONS', 'project and configuration require an angular.json workspace; omit them for an explicit tsconfig');
  }
  result.tsconfig = relative(configPath);
  const config = compiler.readConfiguration(configPath, {noEmit: true});
  result.strictTemplates = config.options.strictTemplates ?? null;
  if (config.options.strictTemplates !== true) {
    result.diagnostics.push({code: 'ANGULAR_STRICT_TEMPLATES', severity: 'warning', message: 'strictTemplates is not explicitly enabled in the effective Angular configuration; full strict template type checking is not certified. Configuration was not modified.'});
  }
  result.diagnostics.push(...config.errors.map(diagnostic));
  if (!config.errors.some(d => d.category === ts.DiagnosticCategory.Error)) {
    // Preserve composite/incremental configuration while preventing all writes,
    // including TypeScript build-info files that can otherwise accompany noEmit.
    const host = compiler.createCompilerHost({options: config.options});
    host.writeFile = () => {};
    const compilation = compiler.performCompilation({rootNames: config.rootNames, options: config.options, host});
    result.typeCheckPerformed = true;
    result.templateTypeCheckPerformed = true;
    result.diagnostics.push(...compilation.diagnostics.map(diagnostic));
  }
} catch (error) {
  if (error.message !== 'harness-angular-reported') {
    result.diagnostics.push({code: 'ANGULAR_COMPILER', severity: 'error', message: String(error.stack || error)});
  }
}
result.passed = result.typeCheckPerformed && !result.diagnostics.some(d => d.severity === 'error');
console.log(JSON.stringify(result));
process.exitCode = result.passed ? 0 : 1;
