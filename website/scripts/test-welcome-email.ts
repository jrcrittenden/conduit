#!/usr/bin/env tsx
/**
 * Test Welcome Email - Send a test welcome email
 *
 * Usage:
 *   pnpm test-welcome-email --to user@example.com
 *   pnpm test-welcome-email --to user@example.com --username fcoury
 */

import 'dotenv/config'
import { Resend } from 'resend'
import { render } from '@react-email/components'
import WelcomeEmail from '../src/emails/WelcomeEmail'

// Parse command line arguments
const args = process.argv.slice(2)
const getArg = (name: string): string | undefined => {
  const index = args.findIndex((a) => a.startsWith(`--${name}`))
  if (index === -1) return undefined
  const arg = args[index]
  if (arg.includes('=')) return arg.split('=')[1]
  return args[index + 1]
}

const to = getArg('to')
const username = getArg('username') || 'TestUser'

if (!to) {
  console.error('Usage: pnpm test-welcome-email --to <email> [--username <github-username>]')
  process.exit(1)
}

const resendKey = process.env.RESEND_API_KEY

if (!resendKey) {
  console.error('Error: RESEND_API_KEY must be set in .env')
  process.exit(1)
}

const resend = new Resend(resendKey)

async function main() {
  console.log('\n📧 Sending test welcome email...\n')
  console.log(`  To: ${to}`)
  console.log(`  GitHub username: ${username}\n`)

  try {
    const html = await render(WelcomeEmail({ githubUsername: username }))

    const { data, error } = await resend.emails.send({
      from: 'Felipe Coury <felipe@getconduit.sh>',
      to: to!,
      subject: 'Welcome to Conduit - Getting Started',
      html,
    })

    if (error) {
      console.error('✗ Failed to send email:', error.message)
      process.exit(1)
    }

    console.log('✓ Email sent successfully!')
    console.log(`  Message ID: ${data?.id}\n`)
  } catch (err) {
    console.error('✗ Error:', err)
    process.exit(1)
  }
}

main()
