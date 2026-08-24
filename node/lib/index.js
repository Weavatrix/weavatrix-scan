'use strict'

const native = require('../index.js')

function encode(options) {
  return options == null ? undefined : JSON.stringify(options)
}

async function scanRepository(root, options) {
  return JSON.parse(await native.scanRepository(root, encode(options)))
}

function scanRepositorySync(root, options) {
  return JSON.parse(native.scanRepositorySync(root, encode(options)))
}

module.exports = { scanRepository, scanRepositorySync }
