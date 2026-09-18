#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const readline = require('readline');

// Paths to version files
const cargoTomlPath = path.join(__dirname, '..', 'src-tauri', 'Cargo.toml');
const packageJsonPath = path.join(__dirname, '..', 'package.json');
const tauriConfPath = path.join(__dirname, '..', 'src-tauri', 'tauri.conf.json');

// Version increment type (patch, minor, major)
const incrementType = process.argv[2] || 'patch';
const autoCommit = process.argv.includes('--commit');
const autoTag = process.argv.includes('--tag');
const autoPush = process.argv.includes('--push');

const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout
});

function question(query) {
  return new Promise(resolve => rl.question(query, resolve));
}

function execCommand(command, description) {
  try {
    console.log(`\n⚙️  ${description}...`);
    const output = execSync(command, { encoding: 'utf8', stdio: 'pipe' });
    if (output.trim()) {
      console.log(output.trim());
    }
    return true;
  } catch (error) {
    console.error(`❌ Failed: ${error.message}`);
    return false;
  }
}

function incrementVersion(version, type) {
  const parts = version.split('.').map(Number);
  
  switch (type) {
    case 'major':
      parts[0]++;
      parts[1] = 0;
      parts[2] = 0;
      break;
    case 'minor':
      parts[1]++;
      parts[2] = 0;
      break;
    case 'patch':
    default:
      parts[2]++;
      break;
  }
  
  return parts.join('.');
}

function updateCargoToml(newVersion) {
  let content = fs.readFileSync(cargoTomlPath, 'utf8');
  content = content.replace(/^version = ".*"$/m, `version = "${newVersion}"`);
  fs.writeFileSync(cargoTomlPath, content, 'utf8');
  console.log(`✓ Updated Cargo.toml to version ${newVersion}`);
}

function updatePackageJson(newVersion) {
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
  packageJson.version = newVersion;
  fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2) + '\n', 'utf8');
  console.log(`✓ Updated package.json to version ${newVersion}`);
}

function updateTauriConf(newVersion) {
  const tauriConf = JSON.parse(fs.readFileSync(tauriConfPath, 'utf8'));
  tauriConf.version = newVersion;
  fs.writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + '\n', 'utf8');
  console.log(`✓ Updated tauri.conf.json to version ${newVersion}`);
}

async function main() {
  try {
    // Read current version from Cargo.toml
    const cargoContent = fs.readFileSync(cargoTomlPath, 'utf8');
    const versionMatch = cargoContent.match(/^version = "(.+)"$/m);
    
    if (!versionMatch) {
      console.error('❌ Could not find version in Cargo.toml');
      process.exit(1);
    }
    
    const currentVersion = versionMatch[1];
    const newVersion = incrementVersion(currentVersion, incrementType);
    
    console.log(`\n🔄 Incrementing version: ${currentVersion} → ${newVersion} (${incrementType})\n`);
    
    // Update all version files
    updateCargoToml(newVersion);
    updatePackageJson(newVersion);
    updateTauriConf(newVersion);
    
    console.log(`\n✅ Version successfully updated to ${newVersion}\n`);
    
    // Interactive git workflow
    if (autoCommit || await question('📝 Commit these changes? (y/n): ') === 'y') {
      const defaultMessage = `chore: bump version to v${newVersion}`;
      let commitMessage = defaultMessage;
      
      if (!autoCommit) {
        const customMessage = await question(`💬 Commit message (press Enter for default):\n   "${defaultMessage}"\n   Custom: `);
        if (customMessage.trim()) {
          commitMessage = customMessage.trim();
        }
      }
      
      if (execCommand('git add .', 'Staging changes')) {
        if (execCommand(`git commit -m "${commitMessage}"`, 'Committing changes')) {
          console.log('✅ Changes committed successfully');
          
          // Create tag
          if (autoTag || await question(`🏷️  Create git tag v${newVersion}? (y/n): `) === 'y') {
            if (execCommand(`git tag v${newVersion}`, `Creating tag v${newVersion}`)) {
              console.log(`✅ Tag v${newVersion} created successfully`);
              
              // Push changes
              if (autoPush || await question('🚀 Push changes and tags to remote? (y/n): ') === 'y') {
                if (execCommand('git push', 'Pushing commits')) {
                  if (execCommand('git push --tags', 'Pushing tags')) {
                    console.log('✅ All changes pushed successfully');
                    
                    console.log('\n📦 Next steps:');
                    console.log(`  1. Build the app: npm run tauri build`);
                    console.log(`  2. Go to: https://github.com/winters27/StreamNook/releases/new?tag=v${newVersion}`);
                    console.log(`  3. Upload: src-tauri/target/release/StreamNook.exe`);
                    console.log(`  4. Publish the release\n`);
                  }
                }
              } else {
                console.log('\n📦 Next steps:');
                console.log('  1. Build the app: npm run tauri build');
                console.log('  2. Push changes: git push && git push --tags');
                console.log(`  3. Create GitHub release: https://github.com/winters27/StreamNook/releases/new?tag=v${newVersion}\n`);
              }
            }
          } else {
            console.log('\n📦 Next steps:');
            console.log('  1. Build the app: npm run tauri build');
            console.log(`  2. Create tag: git tag v${newVersion}`);
            console.log('  3. Push changes: git push && git push --tags');
            console.log(`  4. Create GitHub release: https://github.com/winters27/StreamNook/releases/new?tag=v${newVersion}\n`);
          }
        }
      }
    } else {
      console.log('\n📦 Next steps:');
      console.log('  1. Build the app: npm run tauri build');
      console.log(`  2. Commit: git add . && git commit -m "chore: bump version to v${newVersion}"`);
      console.log(`  3. Tag: git tag v${newVersion}`);
      console.log('  4. Push: git push && git push --tags');
      console.log(`  5. Create GitHub release: https://github.com/winters27/StreamNook/releases/new?tag=v${newVersion}\n`);
    }
    
    rl.close();
    
  } catch (error) {
    console.error('❌ Error:', error.message);
    rl.close();
    process.exit(1);
  }
}

main();
