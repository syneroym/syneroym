# Hub text in Hindi. Same ids as en.ftl; a message missing here is shown in
# English. Use plain, common words. Keep product names (Roym, SynOrg, DID)
# in Latin letters.
#
# A message that shows a service's own English text puts it last, after
# `विवरण:`, and never as the main sentence.
#
# owner review: done 2026-11-07

language-picker-label = भाषा
language-not-saved = यह ब्राउज़र हब को भाषा सहेजने नहीं देता, इसलिए भाषा वैसी ही रहेगी।

age-moments = अभी-अभी

money-input-invalid = "{ $input }" इस मुद्रा के लिए सही राशि नहीं है, जैसे { $example }

error-service-unavailable = यह सेवा अभी उपलब्ध नहीं है। विवरण: { $detail }
error-service-timeout = सेवा ने समय पर जवाब नहीं दिया। विवरण: { $detail }
error-permission = आपको इसकी अनुमति नहीं है। विवरण: { $detail }
error-quota = सीमा पूरी हो गई है। विवरण: { $detail }
error-not-signed-in = आप साइन इन नहीं हैं। विवरण: { $detail }
error-not-owner = यह काम केवल मालिक कर सकता है। विवरण: { $detail }
error-no-owner = इस इंस्टॉलेशन का अभी कोई मालिक नहीं है। विवरण: { $detail }
error-not-local = इस इंस्टॉलेशन ने ऐसा अनुरोध मना कर दिया जो आपकी ओर से नहीं आया था
error-method-not-found = यह अनुरोध इस इंस्टॉलेशन में नहीं मिला। विवरण: { $detail }
error-invalid-input = दी गई जानकारी सही नहीं है। विवरण: { $detail }
error-internal = सेवा में गड़बड़ी हुई। विवरण: { $detail }
error-service = अनुरोध पूरा नहीं हो सका। विवरण: { $detail }
error-network = इस इंस्टॉलेशन तक नहीं पहुँच सके। विवरण: { $detail }
error-http = सर्वर से त्रुटि मिली (HTTP { $code })

login-failed = साइन इन नहीं हो सका: { $detail }
directory-search-failed = खोज नहीं हो सकी: { $detail }। फिर कोशिश करने के लिए "{ $button }" दबाएँ।
directory-search-not-started = यह इंस्टॉलेशन खोज शुरू नहीं कर सका। फिर कोशिश करने के लिए "{ $button }" दबाएँ।
messages-quote-expired = यह कोटेशन समाप्त हो गया है। नया कोटेशन माँगें।
messages-accept-failed = स्वीकार नहीं हो सका: { $detail }
contacts-upsert-failed = संपर्क सहेजा नहीं जा सका: { $detail }
safety-block-failed = ब्लॉक नहीं हो सका: { $detail }
safety-report-failed = रिपोर्ट नहीं भेजी जा सकी: { $detail }
safety-save-failed = सहेजा नहीं जा सका: { $detail }
backup-export-failed = एक्सपोर्ट नहीं हो सका: { $detail }
backup-import-failed = इम्पोर्ट नहीं हो सका: { $detail }
profile-error = त्रुटि: { $detail }

# Sentences that the Rust side also holds. Each one means the same as the
# English one in en.ftl.
payment-notice = यह उस बात को दर्ज करता है जो हर पक्ष भुगतान के बारे में कहता है। Roym पैसे का लेन-देन नहीं देखता और यह पुष्टि नहीं कर सकता कि भुगतान हुआ।
progress-notice = यह स्थिति उस व्यक्ति के सिस्टम से आती है जिसने कोटेशन लिखा। यह किसी भी व्यक्ति का हस्ताक्षरित बयान नहीं है।
payment-claimed = भुगतान करने वाले का कहना है कि उसने भुगतान कर दिया।
payment-acknowledged = पैसे पाने वाला पुष्टि करता है कि उसे भुगतान मिल गया।
fulfilment-claimed = काम करने वाले का कहना है कि काम पूरा हो गया।
fulfilment-acknowledged = काम पाने वाला पुष्टि करता है कि काम पूरा हो गया।
track-unconfirmed = समय-सीमा खत्म होने से पहले कोई पुष्टि दर्ज नहीं हुई।
one-payment-notice = इस कोटेशन का भुगतान एक ही बार में होता है। अग्रिम और किस्तों में भुगतान की सुविधा नहीं है।

group-owner-can-read-notice = इस समूह का मालिक समूह की कुंजी बनाता और साझा करता है, इसलिए जब तक वह मालिक है, वह भेजा गया हर संदेश पढ़ सकता है। किसी सदस्य को जोड़ना या हटाना समूह में सभी को दिखता है।
group-key-trust-notice = हर सदस्य के संदेश उस कुंजी से हस्ताक्षरित होते हैं जो इस इंस्टॉलेशन ने सदस्य के जुड़ते समय पहली बार देखी थी। यह हस्ताक्षरित रिकॉर्ड से कमज़ोर जाँच है, इसलिए यहाँ के संदेश कभी "सत्यापित" नहीं माने जाते।
group-delivery-notice = "अभी हर सदस्य तक नहीं पहुँचा" का मतलब है कि कम से कम एक सदस्य को यह सीधे नहीं मिला। सदस्य आपस में भी संदेश भेजते हैं, इसलिए हो सकता है कि वह इसे बाद में पा ले।
group-join-boundary-notice = आप जुड़ने के बाद भेजे गए संदेश पढ़ सकते हैं। जुड़ने से पहले के संदेश आपके साथ साझा नहीं किए जाते।
group-removed-notice = आपको इस समूह से हटा दिया गया है। जो संदेश आपको पहले मिले थे, उन्हें आप अब भी पढ़ सकते हैं। आप नए संदेश न पढ़ सकते हैं, न भेज सकते हैं।
group-restored-notice = इस समूह का इतिहास बैकअप से वापस लाया गया है। यह इंस्टॉलेशन अब सदस्य नहीं है, इसलिए यहाँ नए संदेश न भेज सकता है, न पा सकता है। मालिक से कहें कि आपका नया पता जोड़ दे।
group-add-unreachable = इस व्यक्ति तक पहुँच नहीं सके, इसलिए उन्हें जोड़ा नहीं जा सका। जिससे आपने पहले बात नहीं की, उसे जोड़ते समय उसका ऑनलाइन होना ज़रूरी है।
group-hidden-notice = छिपाया हुआ समूह दिखाया नहीं जाता। नए संदेश 30 दिन तक रखे जाते हैं, और जब तक मालिक आपको हटा न दे, आप सदस्य बने रहते हैं।
group-transcript-check-notice = जिन सदस्यों को एक ही कोड दिखता है, उनके पास वही संदेश उसी क्रम में हैं।
group-cards-not-allowed = कार्ड केवल आमने-सामने की (1:1) बातचीत में भेजे जाते हैं।

group-session-ended-notice = आपका सत्र खत्म हो गया, जैसे कि इंस्टॉलेशन दोबारा चालू होने से। फिर से लॉग इन करें।
group-delivery-pending = अभी हर सदस्य तक नहीं पहुँचा
group-delivery-delivered = हर सदस्य तक पहुँच गया
group-delivery-failed = हर सदस्य तक नहीं पहुँचा
group-event-created = { $author } ने समूह बनाया
group-event-added = { $author } ने { $subject } को जोड़ा
group-event-removed = { $author } ने { $subject } को हटाया
group-event-updated = { $author } ने समूह में बदलाव किया

record-text-by-author = लेखक द्वारा { $language } से अनुवादित।
record-text-original = मूल विवरण, { $language } में।
record-text-original-binds = मूल विवरण, { $language } में, ही मान्य होगा।
record-text-show-original = मूल विवरण दिखाएँ
record-text-show-translation = लेखक का अनुवाद दिखाएँ
chat-text-not-stated = भाषा नहीं बताई गई

card-unknown-title = अज्ञात कार्ड ({ $type } v{ $version })
card-unknown-body = यह क्लाइंट इस कार्ड के प्रकार या संस्करण को नहीं समझता।
card-refused-title = असत्यापित { $type } कार्ड (v{ $version })
card-refused-notice = इस नोड को इस प्रकार का एक कार्ड मिला, पर वह उसे सत्यापित नहीं कर सका। यह इसलिए दिखाया गया है कि आपको पता चले कि वह भेजा गया था, पर उसे स्वीकार नहीं किया जा सकता और उस पर कोई कार्रवाई नहीं की जा सकती।
card-refused-reason = कारण: { $reason }

terms-scope-label = काम का ब्योरा:
terms-total = कुल: { $amount }
terms-tax = कर: { $amount }
terms-fees = शुल्क: { $amount }
terms-timing-before = काम से पहले भुगतान
terms-timing-after = काम के बाद भुगतान
terms-payee-label = भुगतान पाने वाला, जैसा इस कोटेशन में तय हुआ:
terms-schedule-label = समय:
terms-range-separator = { " - " }
terms-location = जगह: { $where }
terms-address-label = इस कोटेशन में दिया गया पता:
terms-cancellation-label = रद्द करने की शर्तें:
terms-refund-label = वापसी की शर्तें:
terms-dispute-label = विवाद सुलझाने का तरीका:

quote-title = कोटेशन
quote-expiry-label = वैधता खत्म होने का समय:
quote-expires-expired = वैधता खत्म होने का समय: { $date } (खत्म हो चुकी)
quote-expired-notice = यह कोटेशन { $date } को खत्म हो गया। नया कोटेशन माँगें।

receipt-title = समझौते की रसीद
receipt-pair-complete = दोनों पक्षों ने इन शर्तों पर हस्ताक्षर कर दिए हैं।
receipt-pair-half-recipient = अभी केवल उस व्यक्ति ने हस्ताक्षर किए हैं जिसे कोटेशन भेजा गया था।
receipt-pair-half-author = अभी केवल कोटेशन लिखने वाले व्यक्ति ने हस्ताक्षर किए हैं।
receipt-role-author = हस्ताक्षर करने वाला: कोटेशन लिखने वाला व्यक्ति
receipt-role-recipient = हस्ताक्षर करने वाला: जिसे कोटेशन भेजा गया वह व्यक्ति
receipt-quote-to = कोटेशन किसे: { $did }
receipt-quote-by = कोटेशन किसका: { $did }
receipt-quote-record = कोटेशन रिकॉर्ड: { $id }
receipt-expiry-label = कोटेशन की वैधता:

request-title = अनुरोध
request-categories = श्रेणियाँ: { $categories }
request-listing = लिस्टिंग: { $id }
request-window-label = समय-सीमा:
request-notice-label = सूचना:

progress-title = बुकिंग की स्थिति
progress-status = स्थिति: { $state }
progress-state-scheduled = तय हुआ
progress-state-in-progress = जारी है
progress-state-completed = पूरा हुआ
progress-state-cancelled = रद्द
progress-state-conflict = टकराव
progress-state-ended-unconfirmed = खत्म, पुष्टि नहीं हुई
progress-conflict-slot-taken = इस समय पर पहले किसी और की बुकिंग हो गई।
progress-conflict-slot-unavailable = लिस्टिंग के मालिक ने यह समय हटा दिया।
progress-cancel-reason = रद्द करने का कारण: { $reason }

payment-request-title = भुगतान का अनुरोध
payment-ack-title = भुगतान की पावती
payment-amount = राशि: { $amount }
payment-note = नोट: { $note }
payment-observed-label = देखा गया:
payment-method = तरीका: { $method }
payment-reference = संदर्भ: { $reference }
payment-corrected = यह एक सुधारा हुआ बयान है।

fulfilment-title = काम पूरा होने की रसीद

place-editor-at-giver = मेरी जगह पर
place-editor-at-receiver = उनकी जगह पर
place-remote = ऑनलाइन (दूर से)
place-card-at-giver = कोटेशन लिखने वाले की जगह पर
place-card-at-receiver = दूसरे पक्ष की जगह पर
audience-anyone = कोई भी
audience-members = सदस्य
audience-referral = सिफ़ारिश से
audience-past-counterparties = जिनके साथ मैंने पहले काम किया है

hub-title = Roym हब
session-logged-in = लॉग इन: { $did } ({ $auth })
session-logout = लॉग आउट
session-anonymous = लॉग इन नहीं है
tab-error = यह टैब लोड नहीं हो सका: { $detail }
tab-profile = प्रोफ़ाइल
tab-contacts = संपर्क
tab-messages = संदेश
tab-groups = समूह
tab-listings = लिस्टिंग
tab-directory = डायरेक्टरी
tab-memberships = सदस्यता
tab-synorg = SynOrg
tab-safety = सुरक्षा
tab-backup = बैकअप
tab-components = कंपोनेंट
login-title = साइन इन
login-auth-unreachable = इस नोड की साइन इन सेवा तक नहीं पहुँच सके।
login-no-method = इस नोड पर ब्राउज़र से साइन इन करने का कोई तरीका चालू नहीं है।
login-intro = `roymctl session delegate --as <name> --registry-url <registry>` से बनी सत्र-कुंजी की फ़ाइल लोड करें। कुंजी इसी ब्राउज़र में रहती है और पेज का कोड उसे वापस नहीं पढ़ सकता।
login-with-stored = इस ब्राउज़र की कुंजी से साइन इन करें
login-signing-in = साइन इन हो रहा है...
login-stored-gone = सहेजी हुई कुंजी नहीं मिली; सत्र-कुंजी की फ़ाइल लोड करें।
login-load-file = सत्र-कुंजी की फ़ाइल लोड करें
gallery-home-title = Roym हब होम
gallery-did = प्रमाणित DID: { $did }
gallery-contacting = प्रोफ़ाइल सेवा से संपर्क हो रहा है...
gallery-ping-response = प्रोफ़ाइल पिंग का जवाब: { $response }
gallery-ping-failed = प्रोफ़ाइल पिंग विफल: HTTP { $status }
gallery-ping-error = प्रोफ़ाइल पिंग में त्रुटि: { $error }
gallery-heading = कार्ड गैलरी (नमूना कार्ड)

date-invalid = अमान्य तारीख

directory-signed-age = हस्ताक्षर: { $age }
synorg-credential-expires = वैधता खत्म: { $date }
synorg-until = { $date } तक
synorg-until-lifted = जब तक हटाया न जाए

## Shared

error-unexpected = कुछ गड़बड़ हो गई। विवरण: { $detail }
error-no-detail = सेवा से त्रुटि मिली

## Sign-in

login-error-odd-hex = हेक्स पाठ में अंकों की संख्या सम नहीं है
login-error-key-length = अस्थायी गुप्त कुंजी 32 बाइट की होनी चाहिए
login-error-challenge = जाँच का अनुरोध नहीं हो सका: HTTP { $status }
login-error-no-assertion = प्रमाणीकरण सेवा ने हस्ताक्षर के लिए कुछ नहीं भेजा
login-error-http = HTTP { $status }
login-error-service = सेवा ने मना कर दिया। विवरण: { $detail }
login-error-methods = तरीकों की सूची नहीं मिली: HTTP { $status }

common-saved = सहेज लिया गया।
common-save-failed = सहेजा नहीं जा सका: { $detail }
common-remove = हटाएँ

## Profile

profile-title = प्रोफ़ाइल
profile-display-name = दिखने वाला नाम:
profile-text-language = भाषा
profile-languages-heading = आपकी भाषाएँ
profile-languages-add = भाषा जोड़ें
profile-languages-move-up = ऊपर लाएँ
profile-languages-remove = हटाएँ
profile-conversation-address = बातचीत का पता:
profile-save = प्रोफ़ाइल सहेजें
profile-saving = सहेजा जा रहा है...
profile-saved = रिकॉर्ड { $record } सहेजा गया (जारी करने वाला: { $issuer })

## Contacts

contacts-title = संपर्क
contacts-search-placeholder = खोजने का नाम
contacts-search-button = खोजें
contacts-search-empty = इस नाम का कोई संपर्क नहीं है।
contacts-load-failed = संपर्क लोड नहीं हो सके।
contacts-add-heading = संपर्क जोड़ें / इम्पोर्ट करें
contacts-did-placeholder = व्यक्ति का DID
contacts-address-placeholder = बातचीत का पता (प्रोफ़ाइल लिफ़ाफ़ा न हो तो ज़रूरी)
contacts-envelope-placeholder = प्रोफ़ाइल लिफ़ाफ़े का JSON यहाँ चिपकाएँ (ज़रूरी नहीं)
contacts-upsert-button = संपर्क जोड़ें या बदलें
contacts-envelope-invalid = प्रोफ़ाइल लिफ़ाफ़ा सही JSON नहीं है

## Set-up

setup-title = हस्ताक्षर प्रमाणपत्र ज़रूरी है
setup-intro = इस इंस्टॉलेशन में रिकॉर्ड पर हस्ताक्षर करने का कोई चालू प्रमाणपत्र दर्ज नहीं है। दर्ज करने के लिए अपने टर्मिनल से `roymctl roym enrol-signing --master <identity> --registry-url <registry>` चलाएँ। ब्राउज़र यह नहीं कर सकता, क्योंकि उसके पास मुख्य कुंजी नहीं होती।
setup-missing = इनके हस्ताक्षर प्रमाणपत्र नहीं मिले: { $services }
setup-check-again = फिर जाँचें

## Backup

backup-title = ऐप डेटा का बैकअप और बहाली
backup-intro = यह ऐप का डेटा एक्सपोर्ट और इम्पोर्ट करता है। यह आपकी पहचान की गुप्त कुंजी को कभी एक्सपोर्ट नहीं करता और न छूता है।
backup-separate-note = नीचे हर बंडल एक्सपोर्ट के समय आपके हस्ताक्षर से बंद होता है, और बदला गया हो तो इम्पोर्ट पर मना कर दिया जाता है। आपकी पहचान समेत सब कुछ का एन्क्रिप्टेड बैकअप `roymctl roym backup create` से बनता है।
backup-bundle-profile = प्रोफ़ाइल, संपर्क और सुरक्षा के रिकॉर्ड
backup-bundle-conversation = बातचीत और संदेश
backup-bundle-catalog = लिस्टिंग और उपलब्ध समय
backup-bundle-transaction = अनुरोध, कोटेशन और समझौते
backup-bundle-directory = इस इंस्टॉलेशन की SynOrg डायरेक्टरी, अगर वह चलाता हो
backup-export = एक्सपोर्ट
backup-import = इम्पोर्ट
backup-reading = फ़ाइल पढ़ी जा रही है...
backup-importing = इम्पोर्ट हो रहा है...
backup-imported = इम्पोर्ट हुए: { $count } रिकॉर्ड।
backup-imported-unknown = बंडल के रिकॉर्ड इम्पोर्ट हुए।

## Safety

safety-title = सुरक्षा और ब्लॉक सूची
safety-policy-unavailable = इस नोड की सुरक्षा नीति लोड नहीं हो सकी।
safety-block-placeholder = ब्लॉक करने के लिए व्यक्ति का DID या पता
safety-block-button = ब्लॉक करें
safety-blocked-line = ब्लॉक किया गया: { $who }
safety-block-list-failed = ब्लॉक सूची लोड नहीं हो सकी।
safety-report-heading = किसी व्यक्ति, लिस्टिंग या संदेश की रिपोर्ट करें
safety-kind-person = व्यक्ति
safety-kind-listing = लिस्टिंग
safety-kind-message = संदेश
safety-subject-placeholder = जिसकी रिपोर्ट है उसका DID या आईडी
safety-category-impersonation = किसी और बनकर पेश आना
safety-category-fraud = धोखाधड़ी
safety-category-harassment = परेशान करना
safety-category-unsafe-service = असुरक्षित सेवा
safety-category-illegal-content = गैरकानूनी सामग्री
safety-details-placeholder = क्या हुआ (ज़रूरी नहीं)
safety-file-report = रिपोर्ट दर्ज करें
safety-report-row = { $category } - { $kind } { $subject } [{ $status }]
safety-status-open = खुली
safety-status-withdrawn = वापस ली गई
safety-withdraw = वापस लें
safety-reports-load-failed = रिपोर्टें लोड नहीं हो सकीं।
safety-subject-required = जिसकी रिपोर्ट है उसे बताना ज़रूरी है।
safety-recorded = दर्ज हुई { $id } ({ $status })
safety-limit-heading = पहली बार संपर्क की सीमा
safety-limit-intro = एक समय-सीमा के अंदर कितने अनजान लोग आपसे बातचीत शुरू कर सकते हैं। चल रही बातचीत पर कभी कोई सीमा नहीं लगती।
safety-window-placeholder = समय-सीमा (सेकंड में)
safety-max-placeholder = हर समय-सीमा में नए संपर्कों की अधिकतम संख्या
safety-save-limit = सीमा सहेजें
safety-limit-load-failed = मौजूदा सीमा लोड नहीं हो सकी।
safety-whole-numbers = दोनों मान पूर्ण संख्याएँ होने चाहिए।

## Listings

listing-screen-title = लिस्टिंग
listing-your-heading = आपकी लिस्टिंग
listing-load-failed = लिस्टिंग लोड नहीं हो सकीं: { $detail }
listing-empty = अभी कोई लिस्टिंग नहीं है।
listing-version = v{ $count }
listing-history-button = इतिहास
listing-history-line = #{ $n } { $id } (इसने { $previous } की जगह ली)
listing-history-first = #{ $n } { $id } (पहला संस्करण)
listing-history-failed = इतिहास लोड नहीं हो सका: { $detail }
listing-withdraw = वापस लें
listing-withdraw-failed = वापस नहीं लिया जा सका: { $detail }
listing-publish-heading = डायरेक्टरी में प्रकाशित करें
listing-publish-button = यहाँ प्रकाशित करें
listing-published = प्रकाशित हो गया।
listing-not-published = प्रकाशित नहीं हुआ: { $reason }
listing-editor-heading = नई लिस्टिंग बनाएँ / बदलें
listing-save-button = लिस्टिंग सहेजें
listing-saved = { $id } सहेजी गई, संस्करण { $version } ({ $record })
listing-availability-heading = उपलब्ध समय (स्लॉट बुकिंग के लिए)
listing-add-slot = स्लॉट जोड़ें
listing-slot-line = { $start } - { $end } (x{ $capacity })
listing-slots-load-failed = स्लॉट लोड नहीं हो सके: { $detail }
listing-verify-heading = किसी की भेजी हुई लिस्टिंग जाँचें
listing-verify-placeholder = लिस्टिंग लिफ़ाफ़े का JSON यहाँ चिपकाएँ
listing-verify-button = जाँचें
listing-verify-bad-json = यह सही JSON नहीं है।
listing-verified = सत्यापित।
listing-verify-issuer = जारी करने वाला: { $issuer }
listing-verify-address = बातचीत का पता: { $address }
listing-verify-status = स्थिति: { $status }
listing-not-verified = सत्यापित नहीं -- अनजान मानें।
listing-check-failed = जाँच नहीं हो सकी: { $detail }
listing-label-title = शीर्षक
listing-label-slug = स्लग
listing-label-summary = सार
listing-label-categories = श्रेणियाँ
listing-label-address = बातचीत का पता
listing-label-status = स्थिति
listing-label-mode = तरीका
listing-label-lead-time = पहले से सूचना (सेकंड)
listing-label-cancellation-window = रद्द करने की अवधि (सेकंड)
listing-label-max-per-booking = हर बुकिंग में अधिकतम
listing-label-currency = मुद्रा
listing-label-model = मूल्य का तरीका
listing-label-amount = राशि
listing-label-fees = शुल्क
listing-label-methods = भुगतान के तरीके
listing-label-payee = भुगतान पाने वाला
listing-label-tax-included = कर शामिल
listing-label-unit = इकाई
listing-label-pack-size = पैक का आकार
listing-label-condition = हालत
listing-label-sku = स्टॉक कोड
listing-label-duration = अवधि (सेकंड)
listing-label-includes = शामिल
listing-label-excludes = शामिल नहीं
listing-label-prerequisites = पहले से ज़रूरी बातें
listing-label-where = कहाँ
listing-label-disclosure = पता कब बताया जाए
listing-label-service-area = सेवा का क्षेत्र
listing-label-open-to = किसके लिए खुला
listing-label-group-did = समूह का DID
listing-label-fulfilment-receipt = काम पूरा होने की रसीद देता है
listing-label-warranty = वारंटी (सेकंड)
listing-label-retention = रखने की अवधि (सेकंड)
listing-label-start = शुरू
listing-label-end = खत्म
listing-label-capacity = क्षमता
listing-text-language = भाषा
listing-translations-heading = आपके अनुवाद
listing-translation-add = अनुवाद जोड़ें
listing-translation-remove = हटाएँ
listing-label-translation-title = इस भाषा में शीर्षक
listing-label-translation-summary = इस भाषा में संक्षिप्त विवरण (वैकल्पिक)
listing-placeholder-slug = स्लग (ज़रूरी नहीं; शीर्षक से बन जाता है)
listing-placeholder-title = शीर्षक
listing-placeholder-summary = सार
listing-placeholder-address = बातचीत का पता (ज़रूरी नहीं; प्रोफ़ाइल से लिया जाता है)
listing-placeholder-lead-time = पहले से सूचना (सेकंड)
listing-placeholder-cancellation-window = रद्द करने की अवधि (सेकंड)
listing-placeholder-max-per-booking = हर बुकिंग में अधिकतम
listing-placeholder-currency = मुद्रा का कोड (ISO-4217, जैसे EUR)
listing-placeholder-amount = राशि, जैसे 35.50
listing-placeholder-fees = शुल्क, जैसे 2.50 (ज़रूरी नहीं)
listing-placeholder-payee = भुगतान पाने वाला (केवल जानकारी के लिए)
listing-placeholder-pack-size = पैक का आकार
listing-placeholder-sku = स्टॉक कोड (ज़रूरी नहीं)
listing-placeholder-duration = अवधि (सेकंड)
listing-placeholder-includes = शामिल बातें, कॉमा से अलग करें
listing-placeholder-excludes = जो शामिल नहीं है, कॉमा से अलग करें
listing-placeholder-prerequisites = पहले से ज़रूरी बातें, कॉमा से अलग करें
listing-placeholder-area = सेवा के क्षेत्र का नाम (ज़रूरी नहीं)
listing-placeholder-group-did = समूह का DID (सदस्यों के लिए ज़रूरी)
listing-placeholder-warranty = वारंटी (सेकंड)
listing-placeholder-retention = रखने की अवधि (सेकंड)
listing-placeholder-capacity = क्षमता
listing-placeholder-directory-did = Roym डायरेक्टरी सेवा का DID
listing-status-active = सक्रिय
listing-status-draft = मसौदा
listing-status-withdrawn = वापस ली गई
listing-mode-enquiry = पूछताछ
listing-mode-slots = स्लॉट
listing-mode-order = ऑर्डर
listing-model-per-hour = प्रति घंटा
listing-model-fixed = तय कीमत
listing-model-per-unit = प्रति इकाई
listing-model-quote-only = केवल कोटेशन
listing-condition-new = नया
listing-condition-used = इस्तेमाल किया हुआ
listing-condition-refurbished = दोबारा तैयार किया हुआ
listing-disclosure-on-agreement = समझौते के बाद
listing-disclosure-public = सबको दिखे
listing-block-booking = बुकिंग
listing-block-payment = भुगतान
listing-block-product = उत्पाद
listing-block-service = सेवा
listing-block-location = जगह
listing-block-relationship = संबंध
listing-block-service-record = सेवा का रिकॉर्ड
listing-field-lead-time = पहले से सूचना का समय
listing-field-cancellation-window = रद्द करने की अवधि
listing-field-max-per-booking = हर बुकिंग में अधिकतम संख्या
listing-field-pack-size = पैक का आकार
listing-field-duration = अवधि
listing-field-warranty = वारंटी
listing-field-retention = रखने की अवधि
listing-field-capacity = क्षमता
listing-error-coordinate = "{ $input }" अंशों में सही निर्देशांक नहीं है
listing-error-whole = { $field } सेकंड या इकाइयों की पूर्ण संख्या होनी चाहिए
listing-error-required = { $field } ज़रूरी है
listing-error-currency = अज्ञात मुद्रा '{ $currency }'
listing-error-amount-required = जब तक मूल्य का तरीका केवल-कोटेशन न हो, राशि ज़रूरी है
listing-error-quote-only-amount = केवल-कोटेशन वाली लिस्टिंग में राशि नहीं होनी चाहिए
listing-error-members-group = केवल-सदस्यों वाली लिस्टिंग के लिए समूह का DID चाहिए
listing-error-title-required = शीर्षक ज़रूरी है
listing-error-translation-title-required = अनुवाद का शीर्षक ज़रूरी है
listing-error-translation-same-language = अनुवाद में लिस्टिंग की अपनी भाषा नहीं हो सकती
listing-error-translation-duplicate = दो अनुवादों में एक ही भाषा नहीं हो सकती
listing-error-slot-times = स्लॉट के लिए शुरू और खत्म होने का समय चाहिए
listing-error-slot-order = स्लॉट का खत्म होने का समय शुरू होने के बाद का होना चाहिए
listing-error-slot-capacity = स्लॉट की क्षमता कम से कम 1 होनी चाहिए

## Messages

messages-title = संदेश
messages-open-placeholder = बातचीत का पता (किसी लिस्टिंग या संपर्क से)
messages-open-button = बातचीत खोलें
messages-open-failed = खोला नहीं जा सका: { $detail }
messages-search-placeholder = अपने संदेश खोजें
messages-search-button = खोजें
messages-search-empty = कोई संदेश नहीं मिला।
messages-search-failed = खोज नहीं हो सकी: { $detail }
common-load-failed = लोड नहीं हो सका: { $detail }
messages-conversations-empty = अभी कोई बातचीत नहीं है।
messages-count = { $count } संदेश
messages-compose-placeholder = संदेश लिखें
messages-send-button = भेजें
messages-send-failed = भेजा नहीं जा सका: { $detail }
messages-languages-label = संदेश की भाषाएँ
messages-languages-add = भाषा जोड़ें
messages-languages-move-up = ऊपर लाएँ
messages-languages-remove = हटाएँ
request-toggle-button = अनुरोध भेजें
request-text-language = भाषा
request-description-placeholder = बताएँ आपको क्या चाहिए
request-listing-placeholder = लिस्टिंग आईडी (ज़रूरी नहीं)
request-data-use-notice = यह अनुरोध आपके हस्ताक्षर से बनता है और आपके चुने हुए व्यक्ति या व्यवसाय को भेजा जाता है। वे इसकी एक प्रति रखते हैं। इसमें आपका दिया हुआ इलाका होता है, आपका सही पता नहीं। पता केवल उस कोटेशन में बताया जाता है जिसे आप स्वीकार करें।
request-submit-button = अनुरोध जमा करें
request-description-required = विवरण देना ज़रूरी है
request-send-failed = अनुरोध भेजा नहीं जा सका: { $detail }
messages-load-failed = संदेश लोड नहीं हो सके: { $detail }
messages-state-pending = बाकी
messages-state-deposited = मेलबॉक्स में रखा गया
messages-state-delivered = पहुँच गया
messages-state-failed = विफल
messages-state-hint-deposited = दूसरा पक्ष ऑफ़लाइन है। संदेश उनके मेलबॉक्स में इंतज़ार कर रहा है।
messages-refused-rate-limited = स्वीकार नहीं किया गया: यह व्यक्ति नए संपर्कों से आने वाले संदेश सीमित रखता है। बाद में फिर कोशिश करें।
messages-refused-other = स्वीकार नहीं किया गया: { $reason }।
quote-superseded-notice = यह कोटेशन आपके अनुरोध के पहले वाले संस्करण का जवाब है।
messages-unfiled-card = दर्ज न हुआ कार्ड
messages-message-deleted = (संदेश हटा दिया गया)
messages-no-body = (कोई पाठ नहीं)
messages-retry-button = फिर भेजें
messages-delete-button = हटाएँ
quote-request-button = इस अनुरोध पर कोटेशन दें
messages-accept-button = ये शर्तें स्वीकार करें
messages-decline-button = अस्वीकार करें
messages-decline-note = यह केवल उसे बदलता है जो आपको दिखता है। दूसरे पक्ष को नहीं बताया जाता, और कोई रिकॉर्ड हस्ताक्षरित नहीं होता। अगर आप चाहते हैं कि उन्हें पता चले, तो उन्हें संदेश भेजें।
messages-decline-note-placeholder = वैकल्पिक नोट (केवल आपके लिए)
messages-decline-confirm = अस्वीकार की पुष्टि करें
messages-decline-failed = अस्वीकार नहीं हो सका: { $detail }
common-cancel = रद्द करें
quote-text-language = भाषा
quote-scope-placeholder = काम का ब्योरा
quote-currency-placeholder = मुद्रा (जैसे EUR, USD)
quote-amount-placeholder = कुल राशि (जैसे 45.00)
quote-tax-placeholder = कर शामिल (ज़रूरी नहीं, जैसे 5.00)
quote-fees-placeholder = शुल्क शामिल (ज़रूरी नहीं, जैसे 0.00)
quote-payee-placeholder = पैसे पाने वाले का नाम या कंपनी
quote-timing-after = काम के बाद भुगतान
quote-timing-before = काम से पहले भुगतान
quote-address-disclosure-notice = यह पता एक हस्ताक्षरित रिकॉर्ड का हिस्सा बन जाता है, जिसे दोनों पक्ष अपने पास रखते हैं और एक्सपोर्ट कर सकते हैं। एक बार हस्ताक्षर हो चुके रिकॉर्ड से इसे हटाया नहीं जा सकता।
quote-address-placeholder = सेवा का पता
quote-cancellation-placeholder = रद्द करने की शर्तें
quote-default-cancellation = पूरी वापसी के लिए 24 घंटे पहले सूचना ज़रूरी है
quote-refund-placeholder = वापसी की शर्तें
quote-default-refund = काम पूरा न होने पर पूरी वापसी
quote-dispute-placeholder = विवाद सुलझाने का तरीका
quote-default-dispute = छोटे दावों की अदालत या आपसी सुलह
quote-hours-placeholder = कितने घंटों में खत्म हो
quote-send-button = कोटेशन भेजें
quote-scope-required = काम का ब्योरा देना ज़रूरी है
quote-currency-required = मुद्रा देना ज़रूरी है
quote-currency-unknown = अज्ञात मुद्रा कोड "{ $currency }"
quote-amount-required = राशि देना ज़रूरी है
quote-payee-required = पैसे पाने वाले का नाम देना ज़रूरी है
quote-failed = कोटेशन नहीं भेजा जा सका: { $detail }
messages-delete-note-sent = स्थानीय प्रति हटा दी जाती है और हटाने का एक रिकॉर्ड रखा जाता है। दूसरे पक्ष को इसे हटाने का अनुरोध भेजा गया है; उनका क्लाइंट इसे मानेगा या नहीं, यह उनका फ़ैसला है, और इसकी जाँच यहाँ से नहीं हो सकती। इस इंस्टॉलेशन के अपने संदेश-भंडार में वह अब भी है जो उसे मिला था।
messages-delete-note-received = स्थानीय प्रति हटा दी जाती है और हटाने का एक रिकॉर्ड रखा जाता है। यह वह संदेश है जो आपको मिला था; दूसरे पक्ष की प्रति उनकी अपनी है।
messages-delete-ask-peer = उनसे भी अपनी प्रति हटाने को कहें
messages-delete-confirm = यह प्रति हटाएँ
messages-delete-failed = हटाया नहीं जा सका: { $detail }

## Groups

group-login-again = फिर से लॉग इन करें
group-message-deleted = यह संदेश हटा दिया गया था।
group-retry-button = फिर भेजें
group-delete-button = हटाएँ
group-profile-named = { $author } ने समूह का नाम “{ $name }” रखा
group-name-placeholder = समूह का नाम
group-new-button = नया समूह
group-name-label = समूह का नाम
group-hidden-summary = छिपाए हुए समूह ({ $count })
group-list-empty = अभी कोई समूह नहीं है।
group-unnamed = बिना नाम का समूह
group-message-count = { $count } संदेश
group-unhide-button = फिर दिखाएँ
group-load-failed = समूह लोड नहीं हो सका: { $detail }
group-compose-placeholder = संदेश लिखें...
group-send-button = भेजें
group-languages-label = संदेश की भाषाएँ
group-languages-add = भाषा जोड़ें
group-languages-move-up = ऊपर लाएँ
group-languages-remove = हटाएँ
group-rename-placeholder = नया नाम
group-rename-button = नाम बदलें
group-owner-line = मालिक: { $owner }
group-owner-line-you = मालिक: { $owner } (आप)
group-members-heading = सदस्य ({ $count })
group-owner-badge = (मालिक)
group-address-placeholder = पता या व्यक्ति का DID
group-add-member-button = सदस्य जोड़ें
group-key-changed = समूह की कुंजी यहाँ बदली: { $date } (चरण { $epoch })
group-transcript-check = संदेश-सूची जाँच: { $digest }
group-sync-button = अभी सिंक करें
group-hide-button = इस समूह को छिपाएँ

## Membership words

membership-no-instant-removal = किसी समूह का फ़ैसला उन प्रतियों तक, जो दूसरे लोगों के पास पहले से हैं, तभी पहुँचता है जब वे अगली बार जाँच करें। कोई यह वादा नहीं कर सकता कि वह हर जगह एक साथ हट जाएगा।
membership-withheld-revocation = यह समूह की प्रकाशित की हुई हर वापसी दिखाता है। जो वापसी समूह ने प्रकाशित नहीं करने का फ़ैसला किया, वह यह नहीं दिखा सकता।
membership-issuer-changed-check = यह डायरेक्टरी अब कहती है कि वह उस समूह से अलग समूह की ओर से बोलती है जो आपने पहले देखा था। उसका जवाब इस्तेमाल नहीं किया गया।
membership-reason-issuer-changed = यह डायरेक्टरी जिस समूह की ओर से बोलती है वह वह नहीं है जो आपने इसे जोड़ते समय पहली बार देखा था
membership-reason-issuer-not-pinned = इस नोड ने अभी दर्ज नहीं किया कि यह डायरेक्टरी किस समूह की ओर से बोलती है
membership-unknown-no-answer = सदस्यता: अज्ञात ({ $source } ने ऐसा कोई जवाब नहीं दिया जिसे यह नोड इस्तेमाल कर सके)
membership-valid = { $name } का सदस्य, आपके नोड पर जाँचा गया। समूह की वापसियाँ जाँची गईं: { $age }।
membership-none = { $source } ने कोई सदस्यता नहीं दिखाई
membership-unknown = सदस्यता: अज्ञात ({ $reason })
membership-refused = सदस्यता का सबूत जाँच में सही नहीं निकला -- इसे अज्ञात मानें
membership-expired = सदस्यता { $date } को खत्म हो गई
membership-out-of-scope = सदस्यता इस लिस्टिंग को शामिल नहीं करती: { $outside }
membership-refusal-none = इस समूह के पास आपकी कोई सदस्यता दर्ज नहीं है।
membership-this-group = इस समूह
membership-publish-not-admitted = इस समूह ने यह लिस्टिंग स्वीकार नहीं की। { $why }
membership-issuer-unrecorded = इस नोड ने दर्ज नहीं किया कि यह डायरेक्टरी किस समूह की ओर से बोलती है।
membership-issuer-pinned = जब आपने इस डायरेक्टरी को जोड़ा, तब इसने जिस समूह की होने की बात कही: { $issuer }
membership-never-checked = कभी जाँची नहीं गई
membership-checked = जाँची गई: { $age }

membership-revoked = सदस्यता रद्द: समूह ने इसे { $date } को वापस ले लिया।{ $has_reason ->
        [yes] { " " }बताया गया कारण: { $reason }
       *[no] { "" }
    }
membership-suspended =
    { $scope ->
        [listing] यह लिस्टिंग
       *[membership] सदस्यता
    } समूह ने अपने नियम "{ $rule }" के तहत { $since } से रोकी है, { $until ->
        [date] { $date } तक
       *[lifted] जब तक समूह इसे हटा न दे
    }।{ $has_reason ->
        [yes] { " " }बताया गया कारण: { $reason }
       *[no] { "" }
    }

## Memberships

memberships-title = सदस्यताएँ
memberships-intro = हर पंक्ति किसी लिस्टिंग के मालिक की किसी समूह में सदस्यता है, जैसा इस नोड ने समूह के अपने हस्ताक्षरित बयानों से निकाला। यह आपके पास रखी प्रति है, ताज़ा जवाब नहीं।
memberships-load-failed = सदस्यताएँ लोड नहीं हो सकीं: { $detail }
memberships-empty = अभी कोई सदस्यता जाँची नहीं गई। जब खोज में किसी ऐसे समूह का नतीजा आएगा जिसका लिस्टिंग का मालिक सदस्य है, तब एक यहाँ दिखेगी।
membership-row-source = समूह की डायरेक्टरी: { $label }
membership-row-member = लिस्टिंग का मालिक: { $did }
membership-last-error = पिछली जाँच नहीं हो सकी: { $error }
membership-check-again = फिर जाँचें
membership-checked-now = अभी जाँचा गया।
membership-unreachable = इस डायरेक्टरी तक नहीं पहुँच सके; यह वही प्रति है जो आपके पास पहले से थी।
membership-check-failed = जाँच नहीं हो सकी: { $detail }

## Directory

directory-check-again = सदस्यता फिर जाँचें
directory-membership-alone = सदस्यता अपने आप में, इस लिस्टिंग से जुड़े बिना: { $words }
directory-check-failed = जाँच नहीं हो सकी: { $detail }
directory-checked-now = अभी जाँचा गया।
directory-unreachable = इस डायरेक्टरी तक नहीं पहुँच सके। जो पहले से रखा है वह दिखा रहे हैं: { $checked }।
directory-source-error-not-started = यह इंस्टॉलेशन व्यस्त था और उसने कॉल शुरू नहीं की
directory-source-error-timed-out = डायरेक्टरी ने समय पर जवाब नहीं दिया
directory-source-error-not-found = उस पते पर कोई डायरेक्टरी जवाब नहीं देती
directory-source-error-refused = डायरेक्टरी ने अनुरोध मना कर दिया
directory-source-error-unreadable = डायरेक्टरी का जवाब पढ़ा नहीं जा सका
directory-title = डायरेक्टरी
directory-intro = डायरेक्टरी वह जगह है जहाँ आप खोजना चुनते हैं। यह ज़रूरी नहीं है: लिस्टिंग के मालिक का भेजा लिंक हो तो आप सीधे उन तक पहुँच सकते हैं। आपका नोड हर नतीजा खुद जाँचता है -- डायरेक्टरी पर कभी भरोसा नहीं किया जाता।
directory-yours-heading = आपकी डायरेक्टरी
directory-load-failed = आपकी डायरेक्टरी लोड नहीं हो सकीं: { $detail }
directory-empty = अभी कोई डायरेक्टरी नहीं जोड़ी गई।
directory-last-error = पिछली गड़बड़ी: { $reason }
directory-answered-recently = हाल में जवाब दिया
directory-remove-failed = हटाया नहीं जा सका: { $detail }
directory-add-heading = डायरेक्टरी जोड़ें
directory-add-help = किसी दोस्त, सिफ़ारिश या जानी-मानी सूची से मिला Roym डायरेक्टरी सेवा का DID यहाँ चिपकाएँ। डायरेक्टरियों की कोई डायरेक्टरी नहीं है।
directory-add-did-placeholder = did:key:... (Roym डायरेक्टरी सेवा का DID)
directory-add-label-placeholder = इसके लिए आपका नाम (ज़रूरी नहीं)
directory-add-button = डायरेक्टरी जोड़ें
directory-added-note = जोड़ दी गई। ध्यान दें: { $probe }।
directory-added-silent = जोड़ दी गई, पर डायरेक्टरी ने अभी की जाँच का जवाब नहीं दिया।
directory-added = जोड़ दी गई।
directory-add-failed = जोड़ी नहीं जा सकी: { $detail }
directory-search-heading = अपनी डायरेक्टरी में खोजें
directory-search-text-placeholder = खोजने के शब्द (ज़रूरी नहीं)
directory-search-help = हर शब्द किसी भी क्रम में शीर्षक, सार, श्रेणियों या लेखक के अनुवाद में होना चाहिए। लैटिन अक्षरों का शब्द देवनागरी या तमिल में लिखा वही शब्द भी ढूँढ लेता है, और आम वर्तनियाँ मिल जाती हैं: "nal wala" से "naal waala" मिल जाता है। कोई रैंकिंग नहीं है और पैसे देकर ऊपर आने की सुविधा नहीं है -- नतीजे सबसे नए पहले दिखते हैं।
directory-search-button = खोजें
directory-searching = खोज रहे हैं...
directory-no-sources = आपने कोई डायरेक्टरी नहीं जोड़ी है। ऊपर एक जोड़ें, या लिस्टिंग के मालिक तक सीधे लिंक से पहुँचें।
directory-searched = { $count } डायरेक्टरी में खोजा गया।
directory-more-than-returned = इस डायरेक्टरी के पास जितने नतीजे उसने लौटाए उनसे ज़्यादा थे
directory-results-heading = नतीजे
directory-no-results = कोई नतीजा नहीं।
directory-untitled = (बिना शीर्षक)
directory-hit-categories = श्रेणियाँ: { $categories }
directory-hit-offered-by = देने वाला: { $issuer }
directory-signature-ok = हस्ताक्षर: आपके नोड पर जाँचा गया
directory-signature-bad = हस्ताक्षर: जाँच में सही नहीं निकला -- इसे अज्ञात मानें
directory-revocation = वापसी: { $status }
directory-revocation-good = ठीक
directory-revocation-unknown = अज्ञात
directory-seen-at = यहाँ दिखा: { $from }
directory-versions-differ = दो डायरेक्टरी इस पर सहमत नहीं हैं कि इस पेशकश का कौन-सा संस्करण मौजूदा है। हस्ताक्षर वाला नया संस्करण दिखाया गया है।
directory-engage-button = इस लिस्टिंग के मालिक को संदेश भेजें
directory-engage-gone = पूरी लिस्टिंग अब इस खोज में नहीं है।
directory-engage-address = बातचीत का पता: { $address } -- संदेश टैब खोलें और इस पते से बातचीत शुरू करें।
directory-engage-failed = खोला नहीं जा सका: { $detail }
directory-truncated = यहाँ दिखाए गए नतीजों से ज़्यादा नतीजे मिले थे।
directory-refused-heading = अस्वीकार किया गया सबूत
directory-refused-intro = एक या अधिक डायरेक्टरी ने ये दिए, पर इनके हस्ताक्षर आपके नोड पर सही नहीं निकले। ये इसलिए दिखाए गए हैं कि आपको पता रहे कि किसी डायरेक्टरी ने ये दिए। इन्हें कभी असली पेशकश नहीं माना जाता।
directory-refused-fallback = हस्ताक्षर सही नहीं निकला
directory-served-by = इनसे मिला: { $sources }
directory-refused-truncated = यहाँ दिखाए गए अस्वीकृत सबूत से ज़्यादा सबूत दिए गए थे।

## SynOrg

synorg-title = आपका SynOrg
synorg-intro = SynOrg वह डायरेक्टरी है जो आप चलाते हैं: लिस्टिंग के मालिक अपनी लिस्टिंग इसमें प्रकाशित करते हैं, और कोई भी इसमें खोज सकता है। सेटिंग बनाते ही यह इंस्टॉलेशन SynOrg बन जाता है।
synorg-settings-load-failed = सेटिंग लोड नहीं हो सकीं: { $detail }
synorg-status-running = यह इंस्टॉलेशन SynOrg "{ $name }" चलाता है।
synorg-status-none = यह इंस्टॉलेशन अभी कोई SynOrg नहीं चलाता।
synorg-settings-edit = सेटिंग बदलें
synorg-settings-create = अपना SynOrg बनाएँ
synorg-name-placeholder = नाम
synorg-rules-placeholder = वे नियम जो कोई व्यक्ति इस समूह पर भरोसा करने से पहले पढ़ता है
synorg-support-placeholder = सहायता के लिए संपर्क
synorg-dispute-placeholder = विवाद कैसे सुलझाया जाता है
synorg-retention-placeholder = प्रकाशन कितने दिन रखा जाता है
synorg-save-button = सेटिंग सहेजें
synorg-retention-invalid = रखने की अवधि दिनों की पूर्ण संख्या होनी चाहिए।
synorg-label-name = नाम
synorg-label-rules = नियम
synorg-label-categories = श्रेणियाँ
synorg-label-support = सहायता के लिए संपर्क
synorg-label-dispute = विवाद सुलझाने का तरीका
synorg-label-retention = रखने की अवधि (दिन)
synorg-limit-heading = प्रकाशन की सीमा
synorg-limit-intro = एक समय-सीमा के अंदर लिस्टिंग का एक मालिक यहाँ कितनी लिस्टिंग प्रकाशित कर सकता है। लिस्टिंग हटाना इस सीमा में कभी नहीं गिना जाता।
synorg-limit-window-placeholder = समय-सीमा (सेकंड में)
synorg-limit-max-placeholder = हर समय-सीमा में प्रकाशनों की अधिकतम संख्या
synorg-limit-save = सीमा सहेजें
synorg-limit-whole = दोनों मान पूर्ण संख्याएँ होने चाहिए।
synorg-roster-heading = सदस्य
synorg-roster-note = यह सदस्य-सूची केवल यहीं रहती है। इसे नेटवर्क पर कभी नहीं भेजा जाता।
synorg-member-did-placeholder = सदस्य का DID
synorg-roster-note-placeholder = नोट (ज़रूरी नहीं)
synorg-roster-add = सदस्य जोड़ें
synorg-roster-not-removed = हटाया नहीं गया: { $detail }
synorg-roster-load-failed = सदस्य लोड नहीं हो सके: { $detail }
synorg-roster-add-failed = जोड़ा नहीं जा सका: { $detail }
synorg-published-heading = यहाँ प्रकाशित
synorg-published-empty = यहाँ अभी कुछ प्रकाशित नहीं हुआ।
synorg-published-by = { $issuer } द्वारा
synorg-unpublished = आगे की खोज से हटा दिया गया
synorg-publications-load-failed = प्रकाशन लोड नहीं हो सके: { $detail }
synorg-members-heading = सदस्यताएँ और फ़ैसले
synorg-members-load-failed = सदस्यताएँ लोड नहीं हो सकीं: { $detail }
synorg-issue-heading = सदस्यता का प्रमाणपत्र जारी करें
synorg-issue-help = लिस्टिंग का मालिक अपना DID आपको इस ऐप के बाहर देता है। आप यहाँ जो प्रमाणपत्र पर हस्ताक्षर करते हैं, उसी से वे इस समूह में प्रकाशित कर पाते हैं, और खोजने वाले का नोड उसी को जाँचता है।
synorg-issue-button = प्रमाणपत्र जारी करें
synorg-issue-enter-did = सदस्य का DID लिखें।
synorg-issue-choose-category = कम से कम एक श्रेणी चुनें।
synorg-issue-days-range = दिन 1 से 730 के बीच की पूर्ण संख्या होने चाहिए।
synorg-issued = जारी हो गया।
synorg-issue-failed = जारी नहीं हुआ: { $detail }
synorg-label-member-did = सदस्य का DID
synorg-covers-label = इनके लिए:
synorg-label-valid-days = मान्यता (दिन)
synorg-suspend-heading = किसी सदस्य को रोकें
synorg-suspend-listing-placeholder = लिस्टिंग आईडी (पूरी सदस्यता रोकनी हो तो खाली छोड़ें)
synorg-suspend-rule-placeholder = आपका कौन-सा नियम
synorg-reason-placeholder = कारण
synorg-suspend-days-placeholder = दिन (खाली = जब तक आप न हटाएँ)
synorg-suspend-button = रोकें
synorg-suspend-required = सदस्य का DID और नियम देना ज़रूरी है।
synorg-suspend-days-min = दिन कम से कम 1 की पूर्ण संख्या होने चाहिए।
synorg-suspended = रोक दिया गया।
synorg-suspend-failed = रोका नहीं गया: { $detail }
synorg-label-one-listing = केवल एक लिस्टिंग
synorg-label-rule = नियम
synorg-label-reason = कारण
synorg-label-how-long = कितने समय के लिए
synorg-credentials-heading = आपके जारी किए हुए प्रमाणपत्र
synorg-credentials-empty = अभी कोई जारी नहीं हुआ।
synorg-credential-covers = इनके लिए: { $categories }
synorg-credential-status-revoked = रद्द
synorg-credential-status-current = चालू
synorg-credential-status-replaced = बदला हुआ
synorg-revoke-button = रद्द करें
synorg-revoke-failed = रद्द नहीं हुआ: { $detail }
synorg-decisions-heading = रोक और रोक हटाना
synorg-decisions-empty = अभी कोई फ़ैसला नहीं है।
synorg-decision-lifted = रोक हटाई
synorg-decision-suspended-membership = पूरी सदस्यता रोकी
synorg-decision-suspended-listing = एक लिस्टिंग रोकी ({ $id })
synorg-decision-rule = नियम: { $rule }
synorg-lift-button = हटाएँ
synorg-lift-failed = हटाया नहीं गया: { $detail }

## Category picker

category-picker-filter-placeholder = श्रेणी खोजने के लिए टाइप करें
category-picker-remove = { $label } हटाएँ
category-picker-own-code = “{ $code }” कोड का उपयोग करें
category-picker-no-match = कोई श्रेणी मेल नहीं खाती।
category-picker-full = आप अधिकतम { $max } श्रेणियाँ चुन सकते हैं।
