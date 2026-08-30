module.exports = {
  root: true,
  parser: '@typescript-eslint/parser',
  plugins: ['@typescript-eslint'],
  extends: ['eslint:recommended', 'plugin:@typescript-eslint/recommended'],
  rules: {
    'no-restricted-properties': [
      'error',
      {
        property: 'innerHTML',
        message: 'Direct assignment to innerHTML is forbidden for security. Use textContent or DOM construction.',
      },
      {
        property: 'outerHTML',
        message: 'Direct assignment to outerHTML is forbidden for security. Use DOM construction.',
      },
    ],
    'no-restricted-syntax': [
      'error',
      {
        selector: "CallExpression[callee.property.name='insertAdjacentHTML']",
        message: 'insertAdjacentHTML is forbidden for security. Use textContent or DOM construction.',
      },
      {
        selector: "CallExpression[callee.property.name='write']",
        message: 'document.write is forbidden for security. Use DOM construction.',
      },
      {
        selector: "CallExpression[callee.property.name='writeln']",
        message: 'document.writeln is forbidden for security. Use DOM construction.',
      },
      {
        selector: "CallExpression[callee.property.name='createContextualFragment']",
        message: 'createContextualFragment is forbidden for security. Use DOM construction.',
      },
    ],
  },
};
