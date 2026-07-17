const fs = require('node:fs')
const path = require('node:path')
const ts = require('typescript')

const root = path.resolve(__dirname, '..')
const sourceRoot = path.join(root, 'src')
const targets = []

function collect(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name)
    if (entry.isDirectory()) collect(fullPath)
    else if (/\.tsx?$/.test(entry.name) && !entry.name.endsWith('.d.ts')) targets.push(fullPath)
  }
}

function isFunctionLike(node) {
  return ts.isFunctionDeclaration(node) ||
    ts.isFunctionExpression(node) ||
    ts.isArrowFunction(node) ||
    ts.isMethodDeclaration(node) ||
    ts.isGetAccessorDeclaration(node) ||
    ts.isSetAccessorDeclaration(node)
}

function hookName(node) {
  if (!ts.isCallExpression(node)) return null
  const expression = node.expression
  const name = ts.isIdentifier(expression)
    ? expression.text
    : ts.isPropertyAccessExpression(expression)
      ? expression.name.text
      : ''
  return /^use[A-Z0-9]/.test(name) ? name : null
}

function containsReturn(node, owner) {
  let found = false
  function visit(child) {
    if (found) return
    if (child !== owner && isFunctionLike(child)) return
    if (ts.isReturnStatement(child)) {
      found = true
      return
    }
    ts.forEachChild(child, visit)
  }
  visit(node)
  return found
}

function collectHooks(node, owner) {
  const hooks = []
  function visit(child) {
    if (child !== owner && isFunctionLike(child)) return
    const name = hookName(child)
    if (name) hooks.push({ node: child, name })
    ts.forEachChild(child, visit)
  }
  visit(node)
  return hooks
}

function collectConditionalHooks(node, owner, conditional, hooks) {
  if (node !== owner && isFunctionLike(node)) return

  const name = hookName(node)
  if (name && conditional) hooks.push({ node, name })

  if (ts.isIfStatement(node)) {
    collectConditionalHooks(node.expression, owner, conditional, hooks)
    collectConditionalHooks(node.thenStatement, owner, true, hooks)
    if (node.elseStatement) collectConditionalHooks(node.elseStatement, owner, true, hooks)
    return
  }
  if (ts.isConditionalExpression(node)) {
    collectConditionalHooks(node.condition, owner, conditional, hooks)
    collectConditionalHooks(node.whenTrue, owner, true, hooks)
    collectConditionalHooks(node.whenFalse, owner, true, hooks)
    return
  }
  if (ts.isBinaryExpression(node) && (
    node.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken ||
    node.operatorToken.kind === ts.SyntaxKind.BarBarToken ||
    node.operatorToken.kind === ts.SyntaxKind.QuestionQuestionToken
  )) {
    collectConditionalHooks(node.left, owner, conditional, hooks)
    collectConditionalHooks(node.right, owner, true, hooks)
    return
  }
  if (ts.isForStatement(node) || ts.isForInStatement(node) || ts.isForOfStatement(node) ||
      ts.isWhileStatement(node) || ts.isDoStatement(node)) {
    ts.forEachChild(node, child => collectConditionalHooks(child, owner, true, hooks))
    return
  }
  if (ts.isSwitchStatement(node)) {
    collectConditionalHooks(node.expression, owner, conditional, hooks)
    collectConditionalHooks(node.caseBlock, owner, true, hooks)
    return
  }

  ts.forEachChild(node, child => collectConditionalHooks(child, owner, conditional, hooks))
}

collect(sourceRoot)

const findings = []
for (const file of targets) {
  const sourceText = fs.readFileSync(file, 'utf8')
  const sourceFile = ts.createSourceFile(
    file,
    sourceText,
    ts.ScriptTarget.Latest,
    true,
    file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  )

  function report(node, message) {
    const { line, character } = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile))
    findings.push(`${path.relative(root, file)}:${line + 1}:${character + 1}: ${message}`)
  }

  function checkFunction(node) {
    if (!node.body || !ts.isBlock(node.body)) return

    let earlierReturnCanSkipRender = false
    for (const statement of node.body.statements) {
      if (earlierReturnCanSkipRender) {
        for (const hook of collectHooks(statement, node)) {
          report(hook.node, `${hook.name} executes only after an earlier return path`)
        }
      }
      if (!ts.isReturnStatement(statement) && containsReturn(statement, node)) {
        earlierReturnCanSkipRender = true
      }
    }

    const conditionalHooks = []
    collectConditionalHooks(node.body, node, false, conditionalHooks)
    for (const hook of conditionalHooks) {
      report(hook.node, `${hook.name} is called from a conditional render path`)
    }
  }

  function visit(node) {
    if (isFunctionLike(node)) checkFunction(node)
    ts.forEachChild(node, visit)
  }

  visit(sourceFile)
}

const uniqueFindings = [...new Set(findings)].sort()
if (uniqueFindings.length > 0) {
  console.error('React Hook order check failed:')
  console.error(uniqueFindings.join('\n'))
  process.exit(1)
}

console.log(`React Hook order check passed (${targets.length} files).`)
