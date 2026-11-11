# Hub text in English. This file is the fallback for every language: a
# message that another language lacks is shown from here.
#
# To add a message: write `id = text` on one line (or an indented block),
# add the same id to every other .ftl file, and call `t("id")` with the id
# as a literal. A message never starts or ends with a space; Fluent removes
# it. Pass codes and ids as strings, because a number argument is shown
# with digit grouping.

language-picker-label = Language
language-not-saved = This browser does not let the Hub save the language, so it stays as it is.

age-moments = moments ago

money-input-invalid = "{ $input }" is not an amount like { $example } for this currency

# Errors. `$detail` is the service's own English text.
error-service-unavailable = { $detail }
error-service-timeout = { $detail }
error-permission = { $detail }
error-quota = { $detail }
error-not-signed-in = { $detail }
error-not-owner = { $detail }
error-no-owner = { $detail }
error-not-local = this installation refused a request that did not come from you
error-method-not-found = { $detail }
error-invalid-input = { $detail }
error-internal = { $detail }
error-service = { $detail }
error-network = Could not reach this installation: { $detail }
error-http = HTTP error { $code }

# Failures shown beside an action. `$detail` is already worded by the Hub.
login-failed = Sign-in failed: { $detail }
directory-search-failed = The search failed: { $detail }. Press { $button } to try again.
directory-search-not-started = This installation could not start the search. Press { $button } to try again.
messages-quote-expired = This quote has expired. Ask for a new one.
messages-accept-failed = Could not accept: { $detail }
contacts-upsert-failed = Upsert failed: { $detail }
safety-block-failed = Block failed: { $detail }
safety-report-failed = Report failed: { $detail }
safety-save-failed = Save failed: { $detail }
backup-export-failed = Export failed: { $detail }
backup-import-failed = Import failed: { $detail }
profile-error = Error: { $detail }

# Sentences that the Rust side also holds (crates/roym_core). A test reads
# this file and checks that each one appears here word for word, on one
# line. Do not wrap them or change a word without changing the Rust side.
payment-notice = This records what each side says about the payment. Roym does not see the money move and cannot confirm that it did.
progress-notice = This status comes from the system of the person who wrote the quote. It is not a signed statement by either person.
payment-claimed = The payer says they paid.
payment-acknowledged = The payee confirms they received the payment.
fulfilment-claimed = The person doing the work says it is done.
fulfilment-acknowledged = The person receiving the work confirms it is done.
track-unconfirmed = No confirmation was recorded before the window closed.
one-payment-notice = This quote is paid in one payment. Deposits and part payments are not supported.

group-owner-can-read-notice = The owner of this group makes and shares the group's key, so the owner can read every message sent while they own it. Adding or removing a member is shown to everyone in the group.
group-key-trust-notice = Each member's messages are signed with a key this installation first saw when they joined. That is a weaker check than a signed record, so messages here are never marked as verified.
group-delivery-notice = "Not yet delivered to every member" means at least one member has not received it directly. Members also pass messages to each other, so a member may still receive it later.
group-join-boundary-notice = You can read messages sent after you joined. Messages from before you joined are not shared with you.
group-removed-notice = You were removed from this group. You can still read what you received before. You cannot read or send new messages.
group-restored-notice = This group's history was restored from a backup. This installation is not a member, so it cannot send or receive new messages here. Ask the owner to add your new address.
group-add-unreachable = Could not reach this person to add them. Someone you have not talked to before must be online when you add them.
group-hidden-notice = A hidden group is not shown. New messages are held for up to 30 days, and you stay a member until the owner removes you.
group-transcript-check-notice = Members who see the same code hold the same messages in the same order.
group-cards-not-allowed = Cards are sent only in a 1:1 conversation.

# Group lines
group-session-ended-notice = Your session ended, for example because this installation restarted. Log in again.
group-delivery-pending = Not yet delivered to every member
group-delivery-delivered = Delivered to every member
group-delivery-failed = Not delivered to every member
group-event-created = { $author } created the group
group-event-added = { $author } added { $subject }
group-event-removed = { $author } removed { $subject }
group-event-updated = { $author } updated the group

record-text-by-author = Translated by the author from { $language }.
record-text-original = Original text, in { $language }.
record-text-original-binds = The original text, in { $language }, is what binds.
record-text-show-original = Show the original
record-text-show-translation = Show the author's translation
chat-text-not-stated = Language not stated

# Cards. A label that is followed by a value of another kind (a link, a
# time) has no `$value`: the card draws the value after it.
card-unknown-title = Unknown Card ({ $type } v{ $version })
card-unknown-body = This client does not understand this card type or version.
card-refused-title = Unverified { $type } card (v{ $version })
card-refused-notice = This node received a card of this type, but could not verify it. It is shown so you know it was sent, but cannot be accepted or acted upon.
card-refused-reason = Reason: { $reason }

# The terms shared by the quote and the agreement receipt
terms-scope-label = Scope:
terms-total = Total: { $amount }
terms-tax = Tax: { $amount }
terms-fees = Fees: { $amount }
terms-timing-before = Payment before the work
terms-timing-after = Payment after the work
terms-payee-label = Payee, as agreed in this quote:
terms-schedule-label = Schedule:
terms-range-separator = { " - " }
terms-location = Location: { $where }
terms-address-label = Address given in this quote:
terms-cancellation-label = Cancellation terms:
terms-refund-label = Refund terms:
terms-dispute-label = Dispute path:

quote-title = Quote
quote-expiry-label = Expires:
quote-expires-expired = Expires: { $date } (expired)
quote-expired-notice = This quote expired on { $date }. Ask for a new one.

receipt-title = Agreement Receipt
receipt-pair-complete = Both parties have signed these terms.
receipt-pair-half-recipient = Only the person the quote was sent to has signed so far.
receipt-pair-half-author = Only the person who wrote the quote has signed so far.
receipt-role-author = Signed by: the person who wrote the quote
receipt-role-recipient = Signed by: the person the quote was sent to
receipt-quote-to = Quote to: { $did }
receipt-quote-by = Quote by: { $did }
receipt-quote-record = Quote record: { $id }
receipt-expiry-label = Quote expiry:

request-title = Request
request-categories = Categories: { $categories }
request-listing = Listing: { $id }
request-window-label = Window:
request-notice-label = Notice:

progress-title = Booking Progress
progress-status = Status: { $state }
progress-state-scheduled = Scheduled
progress-state-in-progress = In progress
progress-state-completed = Completed
progress-state-cancelled = Cancelled
progress-state-conflict = Conflict
progress-state-ended-unconfirmed = Ended, unconfirmed
progress-conflict-slot-taken = Another booking took this slot first.
progress-conflict-slot-unavailable = The listing owner removed this slot.
progress-cancel-reason = Cancellation reason: { $reason }

payment-request-title = Payment Request
payment-ack-title = Payment Acknowledgement
payment-amount = Amount: { $amount }
payment-note = Note: { $note }
payment-observed-label = Observed:
payment-method = Method: { $method }
payment-reference = Reference: { $reference }
payment-corrected = This is a corrected statement.

fulfilment-title = Fulfilment Receipt

# Places and audiences of a listing
place-editor-at-giver = At my place
place-editor-at-receiver = At their place
place-remote = Remote
place-card-at-giver = At the quote author's place
place-card-at-receiver = At the counterparty's place
audience-anyone = Anyone
audience-members = Members
audience-referral = Referral
audience-past-counterparties = People I have worked with before

# The Hub shell: title, session bar, tabs, sign-in box and card gallery
hub-title = Roym Hub
session-logged-in = Logged in: { $did } ({ $auth })
session-logout = Log out
session-anonymous = Not logged in
tab-error = Could not load this tab: { $detail }
tab-profile = Profile
tab-contacts = Contacts
tab-messages = Messages
tab-groups = Groups
tab-listings = Listings
tab-directory = Directory
tab-memberships = Memberships
tab-synorg = SynOrg
tab-safety = Safety
tab-backup = Backup
tab-components = Components
login-title = Sign in
login-auth-unreachable = Could not reach the auth service on this node.
login-no-method = This node has no browser login method enabled.
login-intro = Load a session key file produced by `roymctl session delegate --as <name> --registry-url <registry>`. The key stays in this browser and cannot be read back by page code.
login-with-stored = Sign in with this browser's key
login-signing-in = Signing in...
login-stored-gone = The stored key is gone; load a session key file.
login-load-file = Load session key file
gallery-home-title = Roym Hub Home
gallery-did = Authenticated DID: { $did }
gallery-contacting = Contacting profile service...
gallery-ping-response = Profile ping response: { $response }
gallery-ping-failed = Profile ping failed: HTTP { $status }
gallery-ping-error = Profile ping error: { $error }
gallery-heading = Card Gallery (Sample Cards)

# Shown where a time from another node is not a date
date-invalid = Invalid Date

# Lines of the Directory and SynOrg screens that show a time
directory-signed-age = Signed { $age }
synorg-credential-expires = expires { $date }
synorg-until = until { $date }
synorg-until-lifted = until lifted

## Shared

error-unexpected = { $detail }
error-no-detail = RPC error

## Sign-in

login-error-odd-hex = odd-length hex string
login-error-key-length = temporary secret key must be 32 bytes
login-error-challenge = challenge failed: HTTP { $status }
login-error-no-assertion = auth service did not return an assertion to sign
login-error-http = HTTP { $status }
login-error-service = { $detail }
login-error-methods = methods failed: HTTP { $status }

common-saved = Saved.
common-save-failed = Save failed: { $detail }
common-remove = Remove

## Profile

profile-title = Profile
profile-display-name = Display Name:
profile-text-language = Text language
profile-languages-heading = Languages you speak
profile-languages-add = Add a language
profile-languages-move-up = Move up
profile-languages-remove = Remove
profile-conversation-address = Conversation Address:
profile-save = Save Profile
profile-saving = Saving...
profile-saved = Saved record { $record } (issuer: { $issuer })

## Contacts

contacts-title = Contacts
contacts-load-failed = Could not load contacts.
contacts-add-heading = Add / Import Contact
contacts-did-placeholder = Person DID
contacts-address-placeholder = Conversation Address (required if no profile envelope)
contacts-envelope-placeholder = Paste Profile Envelope JSON (optional)
contacts-upsert-button = Upsert Contact
contacts-envelope-invalid = Profile envelope is not valid JSON

## Set-up

setup-title = Signing Certificate Required
setup-intro = This installation has no active record-signing certificate enrolled. To enrol, run `roymctl roym enrol-signing --master <identity> --registry-url <registry>` from your terminal. The browser cannot do this because it holds no master key.
setup-missing = Missing signing certificates for: { $services }
setup-check-again = Check again

## Backup

backup-title = App Data Backup & Restore
backup-intro = This exports and imports application data. It never exports or touches your identity secret key.
backup-separate-note = Each bundle below is signed by you when it is exported, and is refused on import if it was changed. An encrypted backup of everything, including your identity, is made with `roymctl roym backup create`.
backup-bundle-profile = Profile, contacts, and safety records
backup-bundle-conversation = Conversations and messages
backup-bundle-catalog = Listings and availability
backup-bundle-transaction = Requests, quotes, and agreements
backup-bundle-directory = This installation's SynOrg directory, if it runs one
backup-export = Export
backup-import = Import
backup-reading = Reading file...
backup-importing = Importing...
backup-imported = Imported: { $count } record(s).
backup-imported-unknown = Imported: bundle record(s).

## Safety

safety-title = Safety & Blocklist
safety-policy-unavailable = Could not load this node's safety policy.
safety-block-placeholder = Person DID or Address to block
safety-block-button = Block
safety-blocked-line = Blocked: { $who }
safety-block-list-failed = Could not load block list.
safety-report-heading = Report a person, listing, or message
safety-kind-person = person
safety-kind-listing = listing
safety-kind-message = message
safety-subject-placeholder = Subject DID or id
safety-category-impersonation = impersonation
safety-category-fraud = fraud
safety-category-harassment = harassment
safety-category-unsafe-service = unsafe-service
safety-category-illegal-content = illegal-content
safety-details-placeholder = What happened (optional)
safety-file-report = File report
safety-report-row = { $category } - { $kind } { $subject } [{ $status }]
safety-status-open = open
safety-status-withdrawn = withdrawn
safety-withdraw = Withdraw
safety-reports-load-failed = Could not load reports.
safety-subject-required = A subject is required.
safety-recorded = Recorded { $id } ({ $status })
safety-limit-heading = First-contact rate limit
safety-limit-intro = How many strangers may start a conversation with you inside one window. An ongoing conversation is never limited.
safety-window-placeholder = window (seconds)
safety-max-placeholder = max new contacts per window
safety-save-limit = Save limit
safety-limit-load-failed = Could not load the current limit.
safety-whole-numbers = Both values must be whole numbers.

## Listings

listing-screen-title = Listings
listing-your-heading = Your listings
listing-load-failed = Could not load listings: { $detail }
listing-empty = No listings yet.
listing-version = v{ $count }
listing-history-button = History
listing-history-line = #{ $n } { $id } (supersedes { $previous })
listing-history-first = #{ $n } { $id } (first version)
listing-history-failed = History failed: { $detail }
listing-withdraw = Withdraw
listing-withdraw-failed = Withdraw failed: { $detail }
listing-publish-heading = Publish to a directory
listing-publish-button = Publish here
listing-published = Published.
listing-not-published = Not published: { $reason }
listing-editor-heading = New / edit a listing
listing-save-button = Save listing
listing-saved = Saved { $id } as version { $version } ({ $record })
listing-availability-heading = Availability (for slot bookings)
listing-add-slot = Add slot
listing-slot-line = { $start } - { $end } (x{ $capacity })
listing-slots-load-failed = Could not load slots: { $detail }
listing-verify-heading = Check a listing someone sent me
listing-verify-placeholder = Paste a listing envelope JSON
listing-verify-button = Check
listing-verify-bad-json = That is not valid JSON.
listing-verified = Verified.
listing-verify-issuer = Issuer: { $issuer }
listing-verify-address = Conversation address: { $address }
listing-verify-status = Status: { $status }
listing-not-verified = Not verified -- treat as unknown.
listing-check-failed = Check failed: { $detail }
listing-label-title = Title
listing-label-slug = Slug
listing-label-summary = Summary
listing-label-categories = Categories
listing-label-address = Conversation address
listing-label-status = Status
listing-label-mode = Mode
listing-label-lead-time = Lead time (s)
listing-label-cancellation-window = Cancellation window (s)
listing-label-max-per-booking = Max per booking
listing-label-currency = Currency
listing-label-model = Model
listing-label-amount = Amount
listing-label-fees = Fees
listing-label-methods = Methods
listing-label-payee = Payee
listing-label-tax-included = tax included
listing-label-unit = Unit
listing-label-pack-size = Pack size
listing-label-condition = Condition
listing-label-sku = SKU
listing-label-duration = Duration (s)
listing-label-includes = Includes
listing-label-excludes = Excludes
listing-label-prerequisites = Prerequisites
listing-label-where = Where
listing-label-disclosure = Address disclosure
listing-label-service-area = Service area
listing-label-open-to = Open to
listing-label-group-did = Group DID
listing-label-fulfilment-receipt = issues a fulfilment receipt
listing-label-warranty = Warranty (s)
listing-label-retention = Retention (s)
listing-label-start = Start
listing-label-end = End
listing-label-capacity = Capacity
listing-text-language = Text language
listing-translations-heading = Translations by you
listing-translation-add = Add a translation
listing-translation-remove = Remove
listing-label-translation-title = Title in this language
listing-label-translation-summary = Summary in this language (optional)
listing-placeholder-slug = slug (optional; derived from the title)
listing-placeholder-title = title
listing-placeholder-summary = summary
listing-placeholder-address = conversation address (optional; taken from your profile)
listing-placeholder-lead-time = lead time (seconds)
listing-placeholder-cancellation-window = cancellation window (seconds)
listing-placeholder-max-per-booking = max per booking
listing-placeholder-currency = currency (ISO-4217, e.g. EUR)
listing-placeholder-amount = amount, e.g. 35.50
listing-placeholder-fees = fees, e.g. 2.50 (optional)
listing-placeholder-payee = payee (informational only)
listing-placeholder-pack-size = pack size
listing-placeholder-sku = SKU (optional)
listing-placeholder-duration = duration (seconds)
listing-placeholder-includes = includes, comma separated
listing-placeholder-excludes = excludes, comma separated
listing-placeholder-prerequisites = prerequisites, comma separated
listing-placeholder-area = service area label (optional)
listing-placeholder-group-did = group DID (required for members)
listing-placeholder-warranty = warranty (seconds)
listing-placeholder-retention = retention (seconds)
listing-placeholder-capacity = capacity
listing-placeholder-directory-did = Roym Directory service DID
listing-status-active = active
listing-status-draft = draft
listing-status-withdrawn = withdrawn
listing-mode-enquiry = enquiry
listing-mode-slots = slots
listing-mode-order = order
listing-model-per-hour = per-hour
listing-model-fixed = fixed
listing-model-per-unit = per-unit
listing-model-quote-only = quote-only
listing-condition-new = new
listing-condition-used = used
listing-condition-refurbished = refurbished
listing-disclosure-on-agreement = on-agreement
listing-disclosure-public = public
listing-block-booking = booking
listing-block-payment = payment
listing-block-product = product
listing-block-service = service
listing-block-location = location
listing-block-relationship = relationship
listing-block-service-record = service_record
listing-field-lead-time = lead time
listing-field-cancellation-window = cancellation window
listing-field-max-per-booking = max per booking
listing-field-pack-size = pack size
listing-field-duration = duration
listing-field-warranty = warranty
listing-field-retention = retention
listing-field-capacity = capacity
listing-error-coordinate = "{ $input }" is not a coordinate in degrees
listing-error-whole = { $field } must be a whole number of seconds or units
listing-error-required = { $field } is required
listing-error-currency = unknown currency '{ $currency }'
listing-error-amount-required = an amount is required unless the model is quote-only
listing-error-quote-only-amount = a quote-only listing must not carry an amount
listing-error-members-group = a members-only listing needs the group DID
listing-error-title-required = a title is required
listing-error-translation-title-required = a translation title is required
listing-error-translation-same-language = a translation cannot use the listing's own language
listing-error-translation-duplicate = two translations cannot use the same language
listing-error-slot-times = a slot needs a start and an end time
listing-error-slot-order = a slot's end must be after its start
listing-error-slot-capacity = a slot needs a capacity of at least 1

## Messages

messages-title = Messages
messages-open-placeholder = A conversation address (from a listing or a contact)
messages-open-button = Open conversation
messages-open-failed = Could not open: { $detail }
messages-search-placeholder = Search your messages
messages-search-button = Search
messages-search-empty = No messages found.
messages-search-failed = Search failed: { $detail }
common-load-failed = Could not load: { $detail }
messages-conversations-empty = No conversations yet.
messages-count = { $count } messages
messages-compose-placeholder = Write a message
messages-send-button = Send
messages-send-failed = Send failed: { $detail }
messages-languages-label = Message languages
messages-languages-add = Add a language
messages-languages-move-up = Move up
messages-languages-remove = Remove
request-toggle-button = Send a request
request-text-language = Text language
request-description-placeholder = Describe what you need
request-listing-placeholder = Listing ID (optional)
# Held by the Rust crate too: a test reads this file and checks this whole
# line. Do not change a word, or wrap the line, without changing the Rust side.
request-data-use-notice = This request is signed by you and sent to the person or business you chose. They keep a copy. It carries the area you gave, not your exact address; an address is disclosed only inside a quote you accept.
request-submit-button = Submit request
request-description-required = Description is required
request-send-failed = Could not send request: { $detail }
messages-load-failed = Could not load messages: { $detail }
messages-state-pending = pending
messages-state-deposited = deposited
messages-state-delivered = delivered
messages-state-failed = failed
messages-state-hint-deposited = The other side is offline. The message waits in their mailbox.
messages-refused-rate-limited = Not accepted: this person limits messages from new contacts. Try again later.
messages-refused-other = Not accepted: { $reason }.
quote-superseded-notice = This quote answers an earlier version of your request.
messages-unfiled-card = Unfiled card
messages-message-deleted = (message deleted)
messages-no-body = (no body)
messages-retry-button = Retry
messages-delete-button = Delete
quote-request-button = Quote this request
messages-accept-button = Accept these terms
messages-decline-button = Decline
messages-decline-note = This only changes what you see. The other side is not told, and no record is signed. Send them a message if you want them to know.
messages-decline-note-placeholder = Optional note (for yourself only)
messages-decline-confirm = Confirm Decline
messages-decline-failed = Decline failed: { $detail }
common-cancel = Cancel
quote-text-language = Text language
quote-scope-placeholder = Scope of work
quote-currency-placeholder = Currency (e.g. EUR, USD)
quote-amount-placeholder = Total amount (e.g. 45.00)
quote-tax-placeholder = Tax included (optional, e.g. 5.00)
quote-fees-placeholder = Fees included (optional, e.g. 0.00)
quote-payee-placeholder = Payee name or company
quote-timing-after = Payment after work
quote-timing-before = Payment before work
# Held by the Rust crate too: a test reads this file and checks this whole
# line. Do not change a word, or wrap the line, without changing the Rust side.
quote-address-disclosure-notice = This address becomes part of a signed record that both parties keep and can export. It cannot be removed from a record already signed.
quote-address-placeholder = Service address
quote-cancellation-placeholder = Cancellation terms
quote-default-cancellation = 24 hours notice required for full refund
quote-refund-placeholder = Refund terms
quote-default-refund = Full refund if work not completed
quote-dispute-placeholder = Dispute path
quote-default-dispute = Small claims court or informal mediation
quote-hours-placeholder = Expires in hours
quote-send-button = Send quote
quote-scope-required = Scope is required
quote-currency-required = Currency is required
quote-currency-unknown = Unknown currency code "{ $currency }"
quote-amount-required = Amount is required
quote-payee-required = Payee is required
quote-failed = Quote failed: { $detail }
# The next two notes are shown before a delete runs. They match the `note`
# that the conversation service returns for `conversation.delete-message`.
# The Hub must not claim that the other side's copy is gone.

messages-delete-note-sent = The local copy is removed and a deletion record kept. A request to delete it was sent to the other side; whether their client honours it is theirs to decide, and this cannot check. This installation's own message store still holds what it received.
messages-delete-note-received = The local copy is removed and a deletion record kept. This is a message you received; the other side's copy is theirs.
messages-delete-ask-peer = Also ask them to delete their copy
messages-delete-confirm = Delete this copy
messages-delete-failed = Delete failed: { $detail }

## Groups

group-login-again = Log in again
group-message-deleted = This message was deleted.
group-retry-button = Retry
group-delete-button = Delete
group-profile-named = { $author } named the group “{ $name }”
group-name-placeholder = Group name
group-new-button = New group
group-name-label = Group Name
group-hidden-summary = Hidden groups ({ $count })
group-list-empty = No groups yet.
group-unnamed = Unnamed group
group-message-count = { $count } messages
group-unhide-button = Unhide
group-load-failed = Could not load group: { $detail }
group-compose-placeholder = Write a message...
group-send-button = Send
group-languages-label = Message languages
group-languages-add = Add a language
group-languages-move-up = Move up
group-languages-remove = Remove
group-rename-placeholder = New name
group-rename-button = Rename
group-owner-line = Owner: { $owner }
group-owner-line-you = Owner: { $owner } (you)
group-members-heading = Members ({ $count })
group-owner-badge = (owner)
group-address-placeholder = Address or Person DID
group-add-member-button = Add member
group-key-changed = Group key changed here: { $date } (epoch { $epoch })
group-transcript-check = Transcript check: { $digest }
group-sync-button = Sync now
group-hide-button = Hide this group

## Membership words

# Held by the Rust crate too: a test reads this file and checks this whole
# line. Do not change a word, or wrap the line, without changing the Rust side.
membership-no-instant-removal = A group's decision reaches copies other people already hold only when they next check. Nobody can promise it is removed everywhere at once.
# Held by the Rust crate too: a test reads this file and checks this whole
# line. Do not change a word, or wrap the line, without changing the Rust side.
membership-withheld-revocation = This shows every withdrawal the group has published. It cannot show one the group chose not to publish.
membership-issuer-changed-check = This directory now says it speaks for a different group than the one you first saw. Its answer was not used.
membership-reason-issuer-changed = the group this directory speaks for is not the one you first saw when you added it
membership-reason-issuer-not-pinned = this node has not yet recorded which group this directory speaks for
membership-unknown-no-answer = Membership: unknown ({ $source } gave no answer this node could use)
membership-valid = Member of { $name }, checked on your node. Group withdrawals checked { $age }.
membership-none = No membership shown by { $source }
membership-unknown = Membership: unknown ({ $reason })
membership-refused = Membership evidence did NOT check out -- treat as unknown
membership-expired = Membership expired on { $date }
membership-out-of-scope = Membership does not cover this listing: { $outside }
membership-refusal-none = This group has no membership on record for you.
membership-this-group = this group
membership-publish-not-admitted = this group did not admit this listing. { $why }
membership-issuer-unrecorded = This node has not recorded which group this directory speaks for.
membership-issuer-pinned = The group this directory said it is when you added it: { $issuer }
membership-never-checked = never checked
membership-checked = checked { $age }

membership-revoked = Membership revoked: the group withdrew it on { $date }.{ $has_reason ->
        [yes] { " " }Reason given: { $reason }
       *[no] { "" }
    }
membership-suspended =
    { $scope ->
        [listing] This listing is suspended
       *[membership] Membership suspended
    } by the group under its rule "{ $rule }", since { $since }, { $until ->
        [date] until { $date }
       *[lifted] until the group lifts it
    }.{ $has_reason ->
        [yes] { " " }Reason given: { $reason }
       *[no] { "" }
    }

## Memberships

memberships-title = Memberships
memberships-intro = Each line is a listing owner's membership in a group, as this node worked it out from the group's own signed statements. It is the copy you hold, not a live answer.
memberships-load-failed = Could not load memberships: { $detail }
memberships-empty = No memberships checked yet. One appears here after a search returns a result from a group the listing owner belongs to.
membership-row-source = Group directory: { $label }
membership-row-member = Listing owner: { $did }
membership-last-error = Last check failed: { $error }
membership-check-again = Check again
membership-checked-now = Checked just now.
membership-unreachable = Could not reach this directory; this is the copy you already held.
membership-check-failed = Could not check: { $detail }

## Directory

directory-check-again = Check membership again
directory-membership-alone = Membership on its own, not tied to this listing: { $words }
directory-check-failed = Could not check: { $detail }
directory-checked-now = Checked just now.
directory-unreachable = Could not reach this directory. Showing what was { $checked }.
directory-source-error-not-started = this installation was busy and did not start the call
directory-source-error-timed-out = the directory did not answer in time
directory-source-error-not-found = no directory answers at that address
directory-source-error-refused = the directory refused the request
directory-source-error-unreadable = the directory's answer could not be read
directory-title = Directories
directory-intro = A directory is a place you choose to search. It is optional: you can always reach a listing owner directly from a link they send you. Your node checks every result itself -- a directory is never trusted.
directory-yours-heading = Your directories
directory-load-failed = Could not load your directories: { $detail }
directory-empty = No directories added yet.
directory-last-error = last error: { $reason }
directory-answered-recently = answered recently
directory-remove-failed = Remove failed: { $detail }
directory-add-heading = Add a directory
directory-add-help = Paste the Roym Directory service DID a friend, a referral or a well-known list gave you. There is no directory of directories.
directory-add-did-placeholder = did:key:... (Roym Directory service DID)
directory-add-label-placeholder = your label for it (optional)
directory-add-button = Add directory
directory-added-note = Added. Note: { $probe }.
directory-added-silent = Added, but the directory did not answer a check just now.
directory-added = Added.
directory-add-failed = Could not add: { $detail }
directory-search-heading = Search your directories
directory-search-text-placeholder = words to look for (optional)
directory-search-help = Matching is on whole words in the title, summary and categories. Punctuation is ignored, so searching for 50% finds 50. There is no ranking and no paid placement -- results are shown newest first.
directory-search-button = Search
directory-searching = Searching...
directory-no-sources = You have not added any directories. Add one above, or reach a listing owner by a direct link.
directory-searched = { $count } directories searched.
directory-more-than-returned = this directory had more matches than it would return
directory-results-heading = Results
directory-no-results = No results.
directory-untitled = (untitled)
directory-hit-categories = Categories: { $categories }
directory-hit-offered-by = Offered by: { $issuer }
directory-signature-ok = signature: checked on your node
directory-signature-bad = signature: did NOT check out -- treat as unknown
directory-revocation = revocation: { $status }
directory-revocation-good = good
directory-revocation-unknown = unknown
directory-seen-at = Seen at: { $from }
directory-versions-differ = Two directories disagree about which version of this offer is current. The newer signed version is shown.
directory-engage-button = Message this listing owner
directory-engage-gone = The full listing is no longer in this run.
directory-engage-address = Conversation address: { $address } -- open the Messages tab and start a conversation with this address.
directory-engage-failed = Could not open: { $detail }
directory-truncated = More results were found than are shown here.
directory-refused-heading = Refused evidence
directory-refused-intro = One or more directories served these, but their signatures did not check out on your node. They are shown so you know a directory served them. They are never treated as real offers.
directory-refused-fallback = signature did not verify
directory-served-by = Served by: { $sources }
directory-refused-truncated = More refused evidence was served than is shown here.

## SynOrg

synorg-title = Your SynOrg
synorg-intro = A SynOrg is a directory you run: listing owners publish their own listings to it, and anyone can search it. Creating settings is what turns this installation into one.
synorg-settings-load-failed = Could not load settings: { $detail }
synorg-status-running = This installation runs the SynOrg "{ $name }".
synorg-status-none = This installation runs no SynOrg yet.
synorg-settings-edit = Edit settings
synorg-settings-create = Create your SynOrg
synorg-name-placeholder = name
synorg-rules-placeholder = the rules a person reads before deciding to trust this group
synorg-support-placeholder = support contact
synorg-dispute-placeholder = how a dispute is handled
synorg-retention-placeholder = how many days a publication is kept
synorg-save-button = Save settings
synorg-retention-invalid = Retention must be a whole number of days.
synorg-label-name = Name
synorg-label-rules = Rules
synorg-label-categories = Categories
synorg-label-support = Support contact
synorg-label-dispute = Dispute path
synorg-label-retention = Retention (days)
synorg-limit-heading = Publication limit
synorg-limit-intro = How many listings one listing owner may publish here inside one window. Taking a listing down never counts against this.
synorg-limit-window-placeholder = window (seconds)
synorg-limit-max-placeholder = max publications per window
synorg-limit-save = Save limit
synorg-limit-whole = Both values must be whole numbers.
synorg-roster-heading = Members
synorg-roster-note = The roster is local. It is never served over the wire.
synorg-member-did-placeholder = member DID
synorg-roster-note-placeholder = note (optional)
synorg-roster-add = Add member
synorg-roster-not-removed = Not removed: { $detail }
synorg-roster-load-failed = Could not load members: { $detail }
synorg-roster-add-failed = Add failed: { $detail }
synorg-published-heading = Published here
synorg-published-empty = Nothing published here yet.
synorg-published-by = by { $issuer }
synorg-unpublished = removed from future search
synorg-publications-load-failed = Could not load publications: { $detail }
synorg-members-heading = Memberships and decisions
synorg-members-load-failed = Could not load memberships: { $detail }
synorg-issue-heading = Issue a membership credential
synorg-issue-help = A listing owner gives you their DID outside this app. The credential you sign here is what lets them publish to this group, and what a searcher's node checks.
synorg-issue-button = Issue credential
synorg-issue-enter-did = Enter the member's DID.
synorg-issue-choose-category = Choose at least one category.
synorg-issue-days-range = Days must be a whole number from 1 to 730.
synorg-issued = Issued.
synorg-issue-failed = Not issued: { $detail }
synorg-label-member-did = Member DID
synorg-covers-label = Covers:
synorg-label-valid-days = Valid for (days)
synorg-suspend-heading = Suspend a member
synorg-suspend-listing-placeholder = listing id (leave empty to suspend the whole membership)
synorg-suspend-rule-placeholder = which of your rules
synorg-reason-placeholder = reason
synorg-suspend-days-placeholder = days (empty = until you lift it)
synorg-suspend-button = Suspend
synorg-suspend-required = A member DID and the rule are required.
synorg-suspend-days-min = Days must be a whole number of at least 1.
synorg-suspended = Suspended.
synorg-suspend-failed = Not suspended: { $detail }
synorg-label-one-listing = One listing only
synorg-label-rule = Rule
synorg-label-reason = Reason
synorg-label-how-long = For how long
synorg-credentials-heading = Credentials you have issued
synorg-credentials-empty = None issued yet.
synorg-credential-covers = covers { $categories }
synorg-credential-status-revoked = revoked
synorg-credential-status-current = current
synorg-credential-status-replaced = replaced
synorg-revoke-button = Revoke
synorg-revoke-failed = Not revoked: { $detail }
synorg-decisions-heading = Suspensions and lifts
synorg-decisions-empty = No decisions yet.
synorg-decision-lifted = lifted a suspension
synorg-decision-suspended-membership = suspended the whole membership
synorg-decision-suspended-listing = suspended one listing ({ $id })
synorg-decision-rule = rule: { $rule }
synorg-lift-button = Lift
synorg-lift-failed = Not lifted: { $detail }

## Category picker

category-picker-filter-placeholder = Type to find a category
category-picker-remove = Remove { $label }
category-picker-own-code = Use the code “{ $code }”
category-picker-no-match = No category matches.
category-picker-full = You can choose up to { $max } categories.
