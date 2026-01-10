#!/usr/bin/env tsx
/**
 * Tail invite acceptances in real-time
 * Usage: pnpm tail-invites
 */
import 'dotenv/config'
import { createClient } from '@supabase/supabase-js'

const supabaseUrl = process.env.PUBLIC_SUPABASE_URL
const supabaseKey = process.env.PUBLIC_SUPABASE_ANON_KEY

if (!supabaseUrl || !supabaseKey) {
  console.error('Error: PUBLIC_SUPABASE_URL and PUBLIC_SUPABASE_ANON_KEY must be set')
  process.exit(1)
}

const supabase = createClient(supabaseUrl, supabaseKey)

const POLL_INTERVAL = 3000 // 3 seconds
let lastAcceptedAt: string | null = null
let seenIds = new Set<string>()

function formatDate(date: string): string {
  return new Date(date).toLocaleString()
}

async function checkNewAcceptances() {
  const query = supabase
    .from('waitlist')
    .select('id, email, github_username, accepted_at, invited_at')
    .not('accepted_at', 'is', null)
    .order('accepted_at', { ascending: false })
    .limit(20)

  const { data, error } = await query

  if (error) {
    console.error('Error querying:', error.message)
    return
  }

  if (!data || data.length === 0) return

  // On first run, just populate seen IDs and show recent
  if (seenIds.size === 0) {
    console.log('\n📋 Recent acceptances:')
    console.log('─'.repeat(70))

    // Show last 5 on startup
    const recent = data.slice(0, 5).reverse()
    for (const entry of recent) {
      seenIds.add(entry.id)
      console.log(`  ✅ ${entry.github_username} (${entry.email})`)
      console.log(`     Accepted: ${formatDate(entry.accepted_at)}`)
    }

    // Add rest to seen without printing
    for (const entry of data.slice(5)) {
      seenIds.add(entry.id)
    }

    console.log('─'.repeat(70))
    console.log('\n👀 Watching for new acceptances... (Ctrl+C to stop)\n')
    return
  }

  // Check for new entries
  for (const entry of data) {
    if (!seenIds.has(entry.id)) {
      seenIds.add(entry.id)
      console.log(`\n🎉 NEW ACCEPTANCE!`)
      console.log(`   GitHub: ${entry.github_username}`)
      console.log(`   Email:  ${entry.email}`)
      console.log(`   Time:   ${formatDate(entry.accepted_at)}`)
    }
  }
}

async function showStats() {
  const { count: totalInvited } = await supabase
    .from('waitlist')
    .select('*', { count: 'exact', head: true })
    .not('invited_at', 'is', null)

  const { count: totalAccepted } = await supabase
    .from('waitlist')
    .select('*', { count: 'exact', head: true })
    .not('accepted_at', 'is', null)

  const { count: totalWaitlist } = await supabase
    .from('waitlist')
    .select('*', { count: 'exact', head: true })

  console.log('\n📊 Invite Stats:')
  console.log(`   Total on waitlist: ${totalWaitlist}`)
  console.log(`   Invites sent:      ${totalInvited}`)
  console.log(`   Accepted:          ${totalAccepted}`)
  console.log(`   Pending:           ${(totalInvited || 0) - (totalAccepted || 0)}`)
}

async function main() {
  console.log('🔄 Conduit Invite Tracker')

  await showStats()
  await checkNewAcceptances()

  // Poll for changes
  setInterval(checkNewAcceptances, POLL_INTERVAL)
}

main().catch(console.error)
