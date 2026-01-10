#!/usr/bin/env tsx
/**
 * Tail all waitlist events in real-time
 * Tracks: signups, invites sent, acceptances
 *
 * Usage: pnpm tail
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

interface WaitlistEntry {
  id: string
  email: string
  twitter_handle: string | null
  github_username: string | null
  created_at: string
  invited_at: string | null
  accepted_at: string | null
}

// Track seen events by type to avoid duplicates
const seenSignups = new Set<string>()
const seenInvites = new Set<string>()
const seenAcceptances = new Set<string>()

let initialized = false

function formatDate(date: string): string {
  return new Date(date).toLocaleString()
}

function formatTime(date: string): string {
  return new Date(date).toLocaleTimeString()
}

async function fetchAll(): Promise<WaitlistEntry[]> {
  const { data, error } = await supabase
    .from('waitlist')
    .select('id, email, twitter_handle, github_username, created_at, invited_at, accepted_at')
    .order('created_at', { ascending: false })
    .limit(100)

  if (error) {
    console.error('Error querying:', error.message)
    return []
  }

  return data || []
}

async function checkEvents() {
  const entries = await fetchAll()

  if (!initialized) {
    // First run - populate seen sets and show recent activity
    console.log('\n📋 Recent Activity:')
    console.log('─'.repeat(70))

    const recentEvents: { time: string; icon: string; message: string }[] = []

    for (const entry of entries) {
      seenSignups.add(entry.id)

      if (entry.invited_at) {
        seenInvites.add(entry.id)
      }

      if (entry.accepted_at) {
        seenAcceptances.add(entry.id)
      }

      // Collect recent events for display
      recentEvents.push({
        time: entry.created_at,
        icon: '📝',
        message: `Signup: ${entry.email}${entry.twitter_handle ? ` (@${entry.twitter_handle})` : ''}`,
      })

      if (entry.invited_at) {
        recentEvents.push({
          time: entry.invited_at,
          icon: '📧',
          message: `Invited: ${entry.email}`,
        })
      }

      if (entry.accepted_at) {
        recentEvents.push({
          time: entry.accepted_at,
          icon: '✅',
          message: `Accepted: ${entry.github_username} (${entry.email})`,
        })
      }
    }

    // Sort by time and show last 10
    recentEvents.sort((a, b) => new Date(b.time).getTime() - new Date(a.time).getTime())
    const recent = recentEvents.slice(0, 10).reverse()

    for (const event of recent) {
      console.log(`  ${event.icon} ${formatTime(event.time)} - ${event.message}`)
    }

    console.log('─'.repeat(70))
    console.log('\n👀 Watching for new events... (Ctrl+C to stop)\n')

    initialized = true
    return
  }

  // Check for new events
  for (const entry of entries) {
    // New signup
    if (!seenSignups.has(entry.id)) {
      seenSignups.add(entry.id)
      console.log(`\n📝 NEW SIGNUP!`)
      console.log(`   Email:   ${entry.email}`)
      if (entry.twitter_handle) {
        console.log(`   Twitter: @${entry.twitter_handle}`)
      }
      console.log(`   Time:    ${formatDate(entry.created_at)}`)
    }

    // New invite sent
    if (entry.invited_at && !seenInvites.has(entry.id)) {
      seenInvites.add(entry.id)
      console.log(`\n📧 INVITE SENT!`)
      console.log(`   Email: ${entry.email}`)
      console.log(`   Time:  ${formatDate(entry.invited_at)}`)
    }

    // New acceptance
    if (entry.accepted_at && !seenAcceptances.has(entry.id)) {
      seenAcceptances.add(entry.id)
      console.log(`\n🎉 INVITE ACCEPTED!`)
      console.log(`   GitHub: ${entry.github_username}`)
      console.log(`   Email:  ${entry.email}`)
      console.log(`   Time:   ${formatDate(entry.accepted_at)}`)
    }
  }
}

async function showStats() {
  const { count: totalWaitlist } = await supabase
    .from('waitlist')
    .select('*', { count: 'exact', head: true })

  const { count: totalInvited } = await supabase
    .from('waitlist')
    .select('*', { count: 'exact', head: true })
    .not('invited_at', 'is', null)

  const { count: totalAccepted } = await supabase
    .from('waitlist')
    .select('*', { count: 'exact', head: true })
    .not('accepted_at', 'is', null)

  const pending = (totalInvited || 0) - (totalAccepted || 0)
  const waiting = (totalWaitlist || 0) - (totalInvited || 0)

  console.log('\n📊 Waitlist Stats:')
  console.log(`   Total signups:     ${totalWaitlist}`)
  console.log(`   Waiting for invite: ${waiting}`)
  console.log(`   Invites sent:      ${totalInvited}`)
  console.log(`   Accepted:          ${totalAccepted}`)
  console.log(`   Pending accept:    ${pending}`)
}

async function main() {
  console.log('🔄 Conduit Waitlist Tracker')

  await showStats()
  await checkEvents()

  // Poll for changes
  setInterval(checkEvents, POLL_INTERVAL)
}

main().catch(console.error)
