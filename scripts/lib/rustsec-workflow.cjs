const installCommand = 'cargo install cargo-audit --version 0.22.2 --locked'
const auditPrefix = 'cargo audit --file src-tauri/Cargo.lock'
const approvedAdvisories = ['RUSTSEC-2024-0429']

function rustsecWorkflow(workflow) {
  const quality = workflow.match(/^  quality:\r?\n([\s\S]*?)(?=^  [\w-]+:|$(?![\s\S]))/m)?.[1]
  if (!quality) throw new Error('RustSec quality job is missing')
  if (/^\s+(?:if|continue-on-error):/m.test(quality)) {
    throw new Error('RustSec quality gate must run unconditionally and fail closed')
  }
  const steps = quality.split(/^      - name: /m)
  const run = name => {
    const step = steps.filter(s => s.split(/\r?\n/)[0] === name)
    if (step.length !== 1) throw new Error(`Expected one RustSec step: ${name}`)
    const commands = [...step[0].matchAll(/^        run: (.+)\r?$/gm)]
    if (commands.length !== 1) throw new Error(`Expected one direct RustSec command: ${name}`)
    return commands[0][1].trim()
  }
  const install = run('Install pinned Rust audit tool')
  const audit = run('Audit Rust dependencies')
  if (install !== installCommand) throw new Error('RustSec tool version and locked installation must stay pinned')
  if (quality.indexOf(installCommand) >= quality.indexOf(auditPrefix)) {
    throw new Error('Install the pinned RustSec auditor before running it')
  }
  if (!/^cargo audit --file src-tauri\/Cargo\.lock(?: --ignore RUSTSEC-\d{4}-\d{4})*$/.test(audit)) {
    throw new Error('RustSec must audit the Tauri lockfile without bypasses or shell fallbacks')
  }
  const advisories = [...audit.matchAll(/--ignore (RUSTSEC-\d{4}-\d{4})/g)].map(m => m[1])
  if (JSON.stringify([...advisories].sort()) !== JSON.stringify(approvedAdvisories)) {
    throw new Error('RustSec exceptions must exactly match the reviewed GLib backport')
  }
  return { advisories }
}

module.exports = { rustsecWorkflow }
