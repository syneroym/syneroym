# Hub text in Hindi. Same ids as en.ftl; a message missing here is shown in
# English. Use plain, common words. Keep product names (Roym, SynOrg, DID)
# in Latin letters.
#
# A message that shows a service's own English text puts it last, after
# `विवरण:`, and never as the main sentence.
#
# owner review: pending

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
directory-search-failed = खोज नहीं हो सकी: { $detail }। फिर कोशिश करने के लिए "Search" दबाएँ।
directory-search-not-started = यह इंस्टॉलेशन खोज शुरू नहीं कर सका। फिर कोशिश करने के लिए "Search" दबाएँ।
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

card-unknown-title = अज्ञात कार्ड ({ $type } v{ $version })
card-unknown-body = यह क्लाइंट इस कार्ड के प्रकार या संस्करण को नहीं समझता।
card-refused-title = असत्यापित { $type } कार्ड (v{ $version })
card-refused-notice = इस नोड को इस प्रकार का एक कार्ड मिला, पर वह उसे सत्यापित नहीं कर सका। यह इसलिए दिखाया गया है कि आपको पता चले कि वह भेजा गया था, पर उसे स्वीकार नहीं किया जा सकता और उस पर कोई कार्रवाई नहीं की जा सकती।
card-refused-reason = कारण: { $reason }

terms-scope = काम का ब्योरा: { $value }
terms-total = कुल: { $amount }
terms-tax = कर: { $amount }
terms-fees = शुल्क: { $amount }
terms-timing-before = काम से पहले भुगतान
terms-timing-after = काम के बाद भुगतान
terms-payee-label = भुगतान पाने वाला, जैसा इस कोटेशन में तय हुआ:
terms-schedule-label = समय:
terms-range-separator = { " - " }
terms-location = जगह: { $where }
terms-address = इस कोटेशन में दिया गया पता: { $address }
terms-cancellation = रद्द करने की शर्तें: { $value }
terms-refund = वापसी की शर्तें: { $value }
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
request-notice = सूचना: { $value }

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
login-auth-unreachable = इस नोड की साइन-इन सेवा तक नहीं पहुँच सके।
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
login-error-challenge = साइन-इन की जाँच शुरू नहीं हो सकी: HTTP { $status }
login-error-no-assertion = प्रमाणीकरण सेवा ने हस्ताक्षर के लिए कुछ नहीं भेजा
login-error-http = साइन-इन नहीं हो सका: HTTP { $status }
login-error-service = साइन-इन को सेवा ने मना कर दिया। विवरण: { $detail }
login-error-methods = साइन-इन के तरीके नहीं मिले: HTTP { $status }

common-saved = सहेज लिया गया।

## Profile

profile-title = प्रोफ़ाइल
profile-display-name = दिखने वाला नाम:
profile-conversation-address = बातचीत का पता:
profile-save = प्रोफ़ाइल सहेजें
profile-saving = सहेजा जा रहा है...
profile-saved = रिकॉर्ड { $record } सहेजा गया (जारी करने वाला: { $issuer })

## Contacts

contacts-title = संपर्क
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
