#include "backend.h"

#include <QJsonArray>
#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QSysInfo>
#include <QThreadPool>
#include <QTimer>

extern "C" {
char *secpar_app_call(const char *request);
void secpar_app_free(char *answer);
}

static QJsonObject call(const QJsonObject &request)
{
    QByteArray text = QJsonDocument(request).toJson(QJsonDocument::Compact);
    char *answer = secpar_app_call(text.constData());
    // The request may hold a password. Do not leave it in freed memory.
    text.fill('\0');
    const QJsonObject parsed = answer ? QJsonDocument::fromJson(answer).object() : QJsonObject();
    secpar_app_free(answer);
    return parsed;
}

Backend::Backend(const QJsonObject &configuration, const QString &computerName, QObject *parent)
    : QObject(parent), m_computer(computerName.isEmpty() ? QSysInfo::machineHostName() : computerName)
{
    QJsonObject request = configuration;
    request["do"] = "configure";
    call(request);
    // The stream runs in its own window. Check now and then whether it is still
    // there, so closing that window returns the computer tile to Connect.
    m_watch = new QTimer(this);
    m_watch->setInterval(2000);
    connect(m_watch, &QTimer::timeout, this, &Backend::pollState);
    // A file being sent, and a file that arrives here, are both shown as soon as
    // the background part reports them.
    m_transfers = new QTimer(this);
    m_transfers->setInterval(1500);
    connect(m_transfers, &QTimer::timeout, this, &Backend::pollTransfers);
    // A still of the connected computer, at most every 15 seconds and only while
    // the Computers page is on screen. The host keeps its own picture for five
    // seconds, so this asks less often than it captures.
    m_preview = new QTimer(this);
    m_preview->setInterval(15000);
    connect(m_preview, &QTimer::timeout, this, &Backend::requestPreviews);
    connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, this, &Backend::shutdown);
    m_updateText = "Not checked";
    refresh();
}

void Backend::ask(const QJsonObject &request, std::function<void(const QJsonObject &)> then, bool busy)
{
    if (busy) {
        m_busy = true;
        emit changed();
    }
    QThreadPool::globalInstance()->start([this, request, then = std::move(then), busy] {
        const QJsonObject answer = call(request);
        QMetaObject::invokeMethod(this, [this, answer, then, busy] {
            if (busy)
                m_busy = false;
            then(answer);
            emit changed();
        }, Qt::QueuedConnection);
    });
}

QString Backend::explain(const QJsonObject &answer)
{
    const QString code = answer["error"].toString();
    // The account service words its own refusals for the person signing in.
    if (!answer["message"].toString().isEmpty())
        return answer["message"].toString();
    if (code == "unreachable")
        return "The account service did not answer. Check the internet connection and try again.";
    if (code == "store_unavailable")
        return "The system's password store could not be used. Unlock it and try again.";
    if (code == "background_not_running")
        return "This computer was not added to the account because Secpar's background part is not running.";
    if (code == "offline")
        return "That computer is off or not connected to the internet.";
    if (code == "not_reachable")
        return "That computer did not answer. Check that both computers are on the same network.";
    if (code == "stream_component_missing" || code == "stream_component_failed" || code == "data_folder_unusable")
        return "The stream could not be started on this computer (" + code + ").";
    if (code == "pairing_refused" || code == "pairing_failed")
        return "That computer did not accept this one (" + code + "). Try again.";
    if (code == "preferences_not_saved" || (code.startsWith("invalid_") && code != "invalid_value"))
        return "That setting could not be saved.";
    if (code == "account_unavailable")
        return "That account is not on the friends list. Check the name and try again.";
    if (code == "cannot_friend_yourself")
        return "That is this account's own name.";
    if (code == "invalid_value")
        return "That permission was not saved. Choose one of the values in the list.";
    if (code == "not_connected")
        return "Connect to a computer first.";
    if (code == "not_a_file")
        return "That file could not be read.";
    if (code == "empty_file")
        return "That file is empty, so there is nothing to send.";
    if (code == "too_large")
        return "Files over 2 GB cannot be sent.";
    if (code == "bad_name")
        return "The other computer cannot save a file with that name.";
    if (code == "not_saved")
        return "The other computer could not save the file.";
    if (code == "not_allowed")
        return "That computer does not take files from this device.";
    if (code == "incoming_request_not_found")
        return "That request is no longer waiting. The list has been refreshed.";
    if (code == "signed_out")
        return QString();
    return "That did not work (" + code + "). Try again.";
}

void Backend::takeState(const QJsonObject &state)
{
    m_loaded = true;
    if (!state["ok"].toBool()) {
        m_message = explain(state);
        return;
    }
    m_background = state["background"].toBool();
    m_signedIn = state["signed_in"].toBool();
    m_handle = state["handle"].toString();
    if (!m_signedIn) {
        m_devices.clear();
        m_friends.clear();
        m_friendRequests.clear();
        m_transferText.clear();
        m_receivedText.clear();
        m_receivedKey.clear();
        m_previews.clear();
        m_previewImages.clear();
        m_transfers->stop();
    } else if (!m_transfers->isActive()) {
        m_transfers->start();
    }
    if (state.contains("preferences"))
        m_preferences = state["preferences"].toObject().toVariantMap();
    const QJsonObject hosting = state["hosting"].toObject();
    m_hosting = hosting["hosts"].toBool();
    m_allowedDevices = hosting["allowed"].toInt();
    const qint64 expires = hosting["access_expires_at"].toVariant().toLongLong();
    m_accessList = expires > 0 ? QDateTime::fromSecsSinceEpoch(expires).toString("d MMM yyyy HH:mm") : "Not fetched";
    m_streaming = m_signedIn && state["streaming"].toBool();
    if (!state["connected_to_host"].toBool()) {
        m_connectedId.clear();
        m_streaming = false;
        m_watch->stop();
    } else if (m_streaming && !m_watch->isActive()) {
        m_watch->start();
    }
}

void Backend::pollState()
{
    if (m_polling || m_connectedId.isEmpty())
        return;
    m_polling = true;
    ask({{"do", "state"}}, [this](const QJsonObject &state) {
        m_polling = false;
        takeState(state);
    }, false);
}

// The last two folders of a path, so the note in the window stays short.
static QString shortFolder(const QString &path)
{
    const QStringList parts = QDir::fromNativeSeparators(path).split('/', Qt::SkipEmptyParts);
    if (parts.size() < 2)
        return path;
    return parts.at(parts.size() - 2) + "/" + parts.last();
}

void Backend::takeTransfers(const QJsonObject &answer)
{
    if (!answer["ok"].toBool()) {
        m_transferText.clear();
        return;
    }
    const QVariantList transfers = answer["transfers"].toArray().toVariantList();
    bool sending = false;
    m_transferText.clear();
    if (!transfers.isEmpty()) {
        const QVariantMap transfer = transfers.last().toMap();
        const QString state = transfer["state"].toString();
        const QString name = transfer["name"].toString();
        const double total = transfer["total"].toDouble();
        const double sent = transfer["sent"].toDouble();
        if (state == "sending") {
            sending = true;
            const int percent = total > 0 ? int(sent * 100 / total) : 0;
            m_transferText = "Sending " + name + " " + QString::number(percent) + "%";
        } else if (state == "sent") {
            m_transferText = "Sent " + name;
        } else {
            m_transferText.clear();
            m_message = "Could not send " + name + ". " + explain(QJsonObject{{"error", transfer["error"].toString()}});
        }
    }
    // A file that arrives here is named once, however often the list is read.
    const QJsonObject received = answer["received"].toObject();
    const QString key = received["name"].toString() + "\n" + received["folder"].toString();
    if (!received.isEmpty() && key != m_receivedKey) {
        m_receivedKey = key;
        m_receivedText = "Received " + received["name"].toString() + " \u2014 " + shortFolder(received["folder"].toString());
    }
    // A file that is being sent is followed more closely than the rest.
    m_transfers->setInterval(sending ? 500 : 1500);
}

void Backend::pollTransfers()
{
    if (!m_signedIn || m_pollingTransfers)
        return;
    m_pollingTransfers = true;
    ask({{"do", "transfers"}}, [this](const QJsonObject &answer) {
        m_pollingTransfers = false;
        takeTransfers(answer);
    }, false);
}

PreviewProvider::PreviewProvider(Backend *backend)
    : QQuickImageProvider(QQuickImageProvider::Image), m_backend(backend)
{
}

QImage PreviewProvider::requestImage(const QString &id, QSize *size, const QSize &requested)
{
    Q_UNUSED(requested);
    // "image://preview/<computer id>/<version>": the version is only there so
    // QML asks again when a new still has arrived.
    const QImage image = m_backend->previewImage(id.section('/', 0, 0));
    if (size)
        *size = image.size();
    return image;
}

QImage Backend::previewImage(const QString &id) const
{
    return m_previewImages.value(id);
}

void Backend::watchPreviews(bool on)
{
    m_previewWatch = on;
    if (!on) {
        m_preview->stop();
        m_previewQueue.clear();
        return;
    }
    if (!m_preview->isActive())
        m_preview->start();
    // Show stills at once rather than after the first interval.
    requestPreviews();
}

void Backend::requestPreviews()
{
    // Every computer the account lists as on is asked for a still, whether or
    // not this window is connected to it. One turn every 15 seconds.
    if (!m_previewWatch || !m_signedIn)
        return;
    const qint64 now = QDateTime::currentSecsSinceEpoch();
    if (m_previewAsked > 0 && now - m_previewAsked < 15)
        return;
    m_previewAsked = now;
    m_previewQueue.clear();
    for (const QVariant &entry : m_devices) {
        const QVariantMap device = entry.toMap();
        // A computer that is off is never asked, and one that is not hosting has
        // nothing to show.
        if (!device["hosts"].toBool() || !device["online"].toBool())
            continue;
        m_previewQueue.append(device);
    }
    askNextPreview();
}

void Backend::askNextPreview()
{
    if (!m_previewWatch || m_previewQueue.isEmpty()) {
        m_previewQueue.clear();
        return;
    }
    const QVariantMap device = m_previewQueue.takeFirst().toMap();
    const QString id = device["id"].toString();
    const QString host = device["fingerprint"].toString();
    const QStringList addresses = device["addresses"].toStringList();
    if (id.isEmpty() || host.isEmpty())
        return askNextPreview();
    QJsonObject request{{"do", "preview"}, {"host", host}, {"max_width", 480}};
    request["addresses"] = QJsonArray::fromStringList(addresses);
    ask(request, [this, id](const QJsonObject &answer) {
        // No still, for whatever reason: the tile keeps its monitor icon.
        if (answer["ok"].toBool()) {
            const QByteArray jpeg = QByteArray::fromBase64(answer["jpeg"].toString().toLatin1());
            QImage image;
            if (!jpeg.isEmpty() && image.loadFromData(jpeg, "JPG")) {
                m_previewImages.insert(id, image);
                m_previews.insert(id, QString("image://preview/%1/%2").arg(id).arg(++m_previewVersion));
            }
        }
        // The next computer in this turn, once this answer is in.
        askNextPreview();
    }, false);
}

void Backend::sendFile(const QUrl &file)
{
    const QString path = file.toLocalFile();
    if (path.isEmpty() || m_connectedId.isEmpty())
        return;
    m_message.clear();
    m_transferText = "Sending " + QFileInfo(path).fileName();
    ask({{"do", "send_file"}, {"path", path}}, [this](const QJsonObject &answer) {
        if (!answer["ok"].toBool()) {
            m_transferText.clear();
            m_message = explain(answer);
        }
        if (!m_transfers->isActive())
            m_transfers->start();
    });
}

void Backend::shutdown()
{
    // The event loop is ending, so this runs here rather than on a worker thread.
    // A stream must not outlive the window that started it.
    m_watch->stop();
    m_transfers->stop();
    m_preview->stop();
    if (m_connectedId.isEmpty())
        return;
    call({{"do", "disconnect"}});
    m_connectedId.clear();
    m_streaming = false;
    m_previews.clear();
    m_previewImages.clear();
}

void Backend::connectTo(const QString &id)
{
    m_message.clear();
    m_connectingId = id;
    ask({{"do", "connect"}, {"id", id}}, [this, id](const QJsonObject &answer) {
        m_connectingId.clear();
        if (answer["ok"].toBool()) {
            m_connectedId = id;
            m_streaming = answer["streaming"].toBool();
            if (m_streaming)
                m_watch->start();
            // Its still comes over this tunnel from the next turn on.
            m_previewAsked = 0;
            requestPreviews();
        } else {
            m_message = explain(answer);
        }
    });
}

void Backend::disconnect()
{
    m_watch->stop();
    m_streaming = false;
    ask({{"do", "disconnect"}}, [this](const QJsonObject &) { m_connectedId.clear(); });
}

void Backend::setPreference(const QString &key, const QVariant &value)
{
    m_message.clear();
    QJsonObject preferences = QJsonObject::fromVariantMap(m_preferences);
    if (key == "fps" || key == "bitrate_kbps")
        preferences[key] = value.toInt();
    else
        preferences[key] = QJsonValue::fromVariant(value);
    ask({{"do", "save_preferences"}, {"preferences", preferences}}, [this](const QJsonObject &answer) {
        if (answer["ok"].toBool())
            m_preferences = answer["preferences"].toObject().toVariantMap();
        else
            m_message = explain(answer);
    });
}

void Backend::loadDevices()
{
    ask({{"do", "devices"}}, [this](const QJsonObject &answer) {
        if (answer["ok"].toBool()) {
            m_devices = answer["devices"].toArray().toVariantList();
            // Stills for the computers that are on, as soon as they are known.
            requestPreviews();
        } else if (answer["error"].toString() == "signed_out") {
            m_signedIn = false;
            m_devices.clear();
        } else {
            m_message = explain(answer);
        }
    });
}

void Backend::loadFriends()
{
    ask({{"do", "friends"}}, [this](const QJsonObject &answer) {
        if (answer["ok"].toBool()) {
            m_friends.clear();
            m_friendRequests.clear();
            const QVariantList all = answer["friends"].toArray().toVariantList();
            for (const QVariant &entry : all) {
                const QVariantMap person = entry.toMap();
                if (person["incoming"].toBool())
                    m_friendRequests.append(person);
                else
                    m_friends.append(person);
            }
        } else if (answer["error"].toString() == "signed_out") {
            m_signedIn = false;
            m_friends.clear();
            m_friendRequests.clear();
        } else {
            m_message = explain(answer);
        }
    });
}

void Backend::refresh()
{
    ask({{"do", "state"}}, [this](const QJsonObject &state) {
        takeState(state);
        if (m_signedIn) {
            loadDevices();
            loadFriends();
        }
    });
}

void Backend::signIn(const QString &email, const QString &password)
{
    m_message.clear();
    const QJsonObject request{{"do", "sign_in"}, {"email", email.trimmed()}, {"password", password},
                              {"computer", m_computer}};
    ask(request, [this](const QJsonObject &state) {
        takeState(state);
        if (!m_signedIn)
            return;
        if (!state["enrolled"].toBool())
            m_message = explain({{"error", state["enrol_error"]}});
        loadDevices();
        loadFriends();
    });
}

void Backend::signOut()
{
    m_message.clear();
    ask({{"do", "sign_out"}}, [this](const QJsonObject &answer) {
        if (answer["ok"].toBool() || answer["error"].toString() == "signed_out") {
            m_watch->stop();
            m_transfers->stop();
            m_streaming = false;
            m_hosting = false;
            m_allowedDevices = 0;
            m_accessList.clear();
            m_signedIn = false;
            m_handle.clear();
            m_transferText.clear();
            m_receivedText.clear();
            m_receivedKey.clear();
            m_previews.clear();
            m_previewImages.clear();
            m_previewQueue.clear();
            m_previewAsked = 0;
            m_devices.clear();
            m_friends.clear();
            m_friendRequests.clear();
        } else {
            m_message = explain(answer);
        }
    });
}

void Backend::removeDevice(const QString &id)
{
    ask({{"do", "remove_device"}, {"id", id}}, [this](const QJsonObject &answer) {
        if (!answer["ok"].toBool())
            m_message = explain(answer);
        loadDevices();
    });
}

void Backend::removeDuplicateDevices()
{
    m_message.clear();
    ask({{"do", "remove_duplicate_devices"}}, [this](const QJsonObject &answer) {
        if (!answer["ok"].toBool())
            m_message = explain(answer);
        loadDevices();
    });
}

void Backend::acceptFriend(const QString &id)
{
    m_message.clear();
    ask({{"do", "accept_friend"}, {"id", id}}, [this](const QJsonObject &answer) {
        if (!answer["ok"].toBool())
            m_message = explain(answer);
        loadFriends();
    });
}

void Backend::removeFriend(const QString &id)
{
    m_message.clear();
    ask({{"do", "remove_friend"}, {"id", id}}, [this](const QJsonObject &answer) {
        if (!answer["ok"].toBool())
            m_message = explain(answer);
        loadFriends();
    });
}

// The update row's wording. It says what happened, never what the new version
// will do.
static QString explainUpdate(const QJsonObject &answer)
{
    const QString code = answer["error"].toString();
    if (code == "unreachable")
        return "The release feed did not answer.";
    if (code == "checksum_mismatch")
        return "The download did not match its checksum. Nothing was installed.";
    if (code == "not_writable")
        return "The update could not be saved on this computer.";
    if (code == "not_appimage")
        return "Start Secpar from the AppImage itself to update it.";
    if (code == "not_supported")
        return "Updates are not available for this system.";
    if (code == "install_failed")
        return "The update could not be started.";
    if (code == "no_update")
        return "Up to date";
    return "The update did not work (" + code + ").";
}

void Backend::checkForUpdates()
{
    m_message.clear();
    m_updateAvailable = false;
    m_updateConfirming = false;
    m_updateVersion.clear();
    m_updateText = "Checking for updates";
    emit changed();
    ask({{"do", "update_check"}}, [this](const QJsonObject &answer) {
        if (!answer["ok"].toBool()) {
            m_updateText = explainUpdate(answer);
            return;
        }
        const QJsonObject available = answer["available"].toObject();
        if (available.isEmpty()) {
            m_updateText = "Up to date";
            return;
        }
        m_updateVersion = available["version"].toString();
        m_updateAvailable = true;
        m_updateText = "Update available v" + m_updateVersion;
    });
}

void Backend::installUpdate()
{
    if (!m_updateAvailable || m_installing)
        return;
    m_installing = true;
    m_updateConfirming = false;
    m_updateText = "Downloading";
    emit changed();
    // Runs off the drawing thread, so the row stays askable while it works.
    ask({{"do", "update_install"}}, [this](const QJsonObject &answer) {
        m_installing = false;
        if (!answer["ok"].toBool()) {
            m_updateText = explainUpdate(answer);
            return;
        }
        m_updateAvailable = false;
        m_updateText = "Restart to update";
        emit changed();
        // The installer or the new AppImage takes over; this window must end
        // first, so its files are free.
        emit restartForUpdate();
    });
}

void Backend::cancelUpdate()
{
    m_updateConfirming = false;
    emit changed();
}

void Backend::addFriend(const QString &handle)
{
    m_message.clear();
    const QString name = handle.trimmed();
    if (name.isEmpty())
        return;
    ask({{"do", "request_friend"}, {"handle", name}}, [this](const QJsonObject &answer) {
        const bool sent = answer["ok"].toBool();
        if (!sent)
            m_message = explain(answer);
        else
            loadFriends();
        emit friendRequestSent(sent);
    });
}

// The service answers the permissions now in force, which may differ from the
// change when one permission decides another (Control decides keyboard and mouse).
void Backend::takeFriendSettings(const QString &id, const QJsonObject &answer)
{
    if (!answer["ok"].toBool()) {
        m_message = explain(answer);
        return;
    }
    const QJsonObject settings = answer["settings"].toObject();
    for (int i = 0; i < m_friends.size(); ++i) {
        QVariantMap person = m_friends.at(i).toMap();
        if (person["id"].toString() != id)
            continue;
        person["input"] = settings["input"].toString();
        person["connect"] = settings["connect"].toString();
        person["trusted"] = settings["connect"].toString() == "always";
        person["keyboard"] = settings["keyboard"].toBool();
        person["mouse"] = settings["mouse"].toBool();
        m_friends[i] = person;
        break;
    }
}

void Backend::saveFriendSettings(const QString &id, const QVariantMap &change)
{
    m_message.clear();
    QJsonObject request{{"do", "save_friend_settings"}, {"id", id}};
    if (change.contains("input"))
        request["input"] = change["input"].toString();
    if (change.contains("connect"))
        request["connect"] = change["connect"].toString();
    if (change.contains("keyboard"))
        request["keyboard"] = change["keyboard"].toBool();
    if (change.contains("mouse"))
        request["mouse"] = change["mouse"].toBool();
    ask(request, [this, id](const QJsonObject &answer) { takeFriendSettings(id, answer); });
}
