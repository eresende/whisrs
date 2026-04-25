import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Pango from 'gi://Pango';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const DBUS_INTERFACE = 'org.whisrs.Overlay';
const DBUS_PATH = '/org/whisrs/Overlay';
const STATE_SIGNAL = 'StateChanged';
const LEVEL_SIGNAL = 'LevelChanged';
const OVERLAY_WIDTH = 250;
const OVERLAY_HEIGHT = 50;

export default class WhisrsOverlayExtension extends Extension {
    enable() {
        this._actor = new St.Widget({
            style_class: 'whisrs-overlay whisrs-overlay-hidden',
            layout_manager: new Clutter.FixedLayout(),
            reactive: false,
            visible: false,
        });

        // Bars (recording)
        this._bars = [];
        this._barsBox = new St.BoxLayout({
            style_class: 'whisrs-overlay-bars',
            y_align: Clutter.ActorAlign.CENTER,
        });
        for (let i = 0; i < 4; i++) {
            const bar = new St.Widget({
                style_class: 'whisrs-overlay-bar',
                y_align: Clutter.ActorAlign.CENTER,
                y_expand: false,
            });
            this._bars.push(bar);
            this._barsBox.add_child(bar);
        }

        // Spinner arc (transcribing) — drawn as a styled widget
        this._spinner = new St.Widget({style_class: 'whisrs-overlay-spinner'});

        // Label
        this._label = new St.Label({
            style_class: 'whisrs-overlay-label',
            text: '',
        });
        this._label.clutter_text.set_ellipsize(Pango.EllipsizeMode.NONE);
        this._label.clutter_text.set_line_wrap(false);

        // Divider + timer (recording only)
        this._divider = new St.Widget({style_class: 'whisrs-overlay-divider'});
        this._timer = new St.Label({
            style_class: 'whisrs-overlay-timer',
            text: '00:00',
        });
        this._timer.clutter_text.set_ellipsize(Pango.EllipsizeMode.NONE);

        this._actor.add_child(this._barsBox);
        this._actor.add_child(this._spinner);
        this._actor.add_child(this._label);
        this._actor.add_child(this._divider);
        this._actor.add_child(this._timer);
        Main.uiGroup.add_child(this._actor);

        this._monitorsChangedId = Main.layoutManager.connect(
            'monitors-changed',
            () => this._position()
        );
        this._allocationChangedId = this._actor.connect(
            'notify::allocation',
            () => this._position()
        );

        this._signalId = Gio.DBus.session.signal_subscribe(
            null, DBUS_INTERFACE, STATE_SIGNAL, DBUS_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _sig, parameters) => {
                const [state] = parameters.deep_unpack();
                this._setState(state);
            }
        );
        this._levelSignalId = Gio.DBus.session.signal_subscribe(
            null, DBUS_INTERFACE, LEVEL_SIGNAL, DBUS_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _sig, parameters) => {
                const [level] = parameters.deep_unpack();
                this._setLevel(level);
            }
        );

        this._position();
    }

    disable() {
        this._stopAnimation();

        if (this._signalId) {
            Gio.DBus.session.signal_unsubscribe(this._signalId);
            this._signalId = 0;
        }
        if (this._levelSignalId) {
            Gio.DBus.session.signal_unsubscribe(this._levelSignalId);
            this._levelSignalId = 0;
        }
        if (this._monitorsChangedId) {
            Main.layoutManager.disconnect(this._monitorsChangedId);
            this._monitorsChangedId = 0;
        }
        if (this._allocationChangedId) {
            this._actor.disconnect(this._allocationChangedId);
            this._allocationChangedId = 0;
        }

        this._actor?.destroy();
        this._actor = null;
        this._bars = [];
        this._barsBox = null;
        this._spinner = null;
        this._label = null;
        this._divider = null;
        this._timer = null;
    }

    _setState(state) {
        if (!this._actor || !this._label)
            return;

        const normalized = String(state).toLowerCase();
        this._actor.remove_style_class_name('whisrs-overlay-recording');
        this._actor.remove_style_class_name('whisrs-overlay-transcribing');
        this._actor.remove_style_class_name('whisrs-overlay-hidden');

        if (normalized === 'recording') {
            this._state = 'recording';
            this._recordingStart = Date.now();
            this._label.text = 'RECORDING';
            this._actor.add_style_class_name('whisrs-overlay-recording');
            this._actor.visible = true;
            this._barsBox.visible = true;
            this._spinner.visible = false;
            this._divider.visible = true;
            this._timer.visible = true;
            this._startAnimation();
            this._position();
        } else if (normalized === 'transcribing') {
            this._state = 'transcribing';
            this._label.text = 'TRANSCRIBING ....';
            this._actor.add_style_class_name('whisrs-overlay-transcribing');
            this._actor.visible = true;
            this._barsBox.visible = false;
            this._spinner.visible = true;
            this._divider.visible = false;
            this._timer.visible = false;
            this._startAnimation();
            this._position();
        } else {
            this._state = 'idle';
            this._label.text = '';
            this._actor.add_style_class_name('whisrs-overlay-hidden');
            this._actor.visible = false;
            this._stopAnimation();
        }
    }

    _position() {
        if (!this._actor)
            return;

        const monitor = Main.layoutManager.primaryMonitor;
        const x = Math.floor(monitor.x + (monitor.width - OVERLAY_WIDTH) / 2);
        const y = Math.floor(monitor.y + monitor.height - OVERLAY_HEIGHT - 60);
        this._actor.set_position(Math.max(monitor.x, x), Math.max(monitor.y, y));
        this._actor.set_size(OVERLAY_WIDTH, OVERLAY_HEIGHT);
        this._layoutChildren();
        this._actor.set_pivot_point(0.5, 0.5);
        this._actor.set_easing_mode(Clutter.AnimationMode.EASE_OUT_QUAD);
        this._actor.set_easing_duration(120);
    }

    _layoutChildren() {
        if (!this._actor || !this._label)
            return;

        const cy = Math.floor(OVERLAY_HEIGHT / 2);

        // Recording layout: [bars] [RECORDING] | [00:00]
        if (this._barsBox) {
            this._barsBox.set_position(24, cy - 16);
            this._barsBox.set_size(36, 32);
        }
        if (this._label) {
            const labelX = this._state === 'transcribing' ? 62 : 70;
            this._label.set_position(labelX, cy - 10);
        }
        if (this._divider) {
            this._divider.set_position(178, cy - 12);
            this._divider.set_size(1, 24);
        }
        if (this._timer)
            this._timer.set_position(188, cy - 10);

        // Transcribing layout: [spinner] [TRANSCRIBING]
        if (this._spinner) {
            this._spinner.set_position(16, cy - 12);
            this._spinner.set_size(24, 24);
        }
    }

    _startAnimation() {
        if (this._animationId)
            return;

        this._frame = 0;
        this._level = 0;
        this._targetLevel = 0;
        this._animationId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 24, () => {
            this._frame++;
            const target = this._targetLevel ?? 0;
            this._level = target > this._level ? target : Math.max(0, this._level * 0.85);
            this._updateBars();
            this._updateTimer();
            this._updateSpinner();
            return GLib.SOURCE_CONTINUE;
        });
        this._updateBars();
    }

    _stopAnimation() {
        if (this._animationId) {
            GLib.Source.remove(this._animationId);
            this._animationId = 0;
        }
    }

    _updateBars() {
        if (!this._bars || this._state !== 'recording')
            return;

        for (let i = 0; i < this._bars.length; i++) {
            const raw = Number.isFinite(this._level) ? this._level : 0;
            const level = raw < 0.1 ? 0 : Math.min(1, (raw - 0.1) / 0.85);
            const variance = 0.6 + (((i * 7 + 3) % 4) / 4) * 0.4;
            const height = 4 + Math.round(Math.min(1, level * variance) * 28);
            this._bars[i].set_height(height);
        }
    }

    _updateTimer() {
        if (!this._timer || this._state !== 'recording' || !this._recordingStart)
            return;

        const elapsed = Math.floor((Date.now() - this._recordingStart) / 1000);
        const mm = String(Math.floor(elapsed / 60)).padStart(2, '0');
        const ss = String(elapsed % 60).padStart(2, '0');
        this._timer.text = `${mm}:${ss}`;
    }

    _updateSpinner() {
        if (!this._spinner || this._state !== 'transcribing')
            return;

        const angle = (this._frame * 8) % 360;
        this._spinner.set_pivot_point(0.5, 0.5);
        this._spinner.set_rotation_angle(Clutter.RotateAxis.Z_AXIS, angle);
    }

    _setLevel(level) {
        const numeric = Number(level);
        if (!Number.isFinite(numeric))
            return;

        this._targetLevel = Math.max(0, Math.min(1, numeric));
        if (this._state === 'recording')
            this._updateBars();
    }
}
