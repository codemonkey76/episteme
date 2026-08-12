import 'package:flutter/material.dart';
import 'package:image_picker/image_picker.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';
import 'package:url_launcher/url_launcher.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/shipments.dart';

const shipmentStatuses = <String, String>{
  'ordered': 'Ordered',
  'in_transit': 'In transit',
  'out_for_delivery': 'Out for delivery',
  'delivered': 'Delivered',
  'exception': 'Problem',
  'cancelled': 'Cancelled',
};

/// Pill colour: blue while moving, green on arrival, amber for trouble.
Color shipmentStatusColor(String status) => switch (status) {
      'delivered' => Palette.ok,
      'exception' => Palette.warn,
      'cancelled' => Palette.faint,
      _ => Palette.accent,
    };

/// "Thu 14 Aug · tomorrow" — an ETA only means something relative to today.
String shipmentEtaText(DateTime? eta) {
  if (eta == null) return 'No ETA';
  final date = DateFormat('EEE d MMM').format(eta);
  final today = DateTime.now();
  final days = DateTime(eta.year, eta.month, eta.day)
      .difference(DateTime(today.year, today.month, today.day))
      .inDays;
  return switch (days) {
    0 => '$date · today',
    1 => '$date · tomorrow',
    -1 => '$date · yesterday',
    _ when days > 1 => '$date · in $days days',
    _ => '$date · ${-days} days ago',
  };
}

/// Ask for a photo source, then return the picked image (null if cancelled).
Future<XFile?> pickShipmentPhoto(BuildContext context) async {
  final source = await showModalBottomSheet<ImageSource>(
    context: context,
    backgroundColor: Palette.surface,
    builder: (_) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(Icons.photo_camera_outlined, color: Palette.muted),
            title: const Text('Take a photo', style: TextStyle(color: Palette.fg)),
            onTap: () => Navigator.pop(context, ImageSource.camera),
          ),
          ListTile(
            leading: const Icon(Icons.photo_library_outlined, color: Palette.muted),
            title: const Text('Choose from gallery', style: TextStyle(color: Palette.fg)),
            onTap: () => Navigator.pop(context, ImageSource.gallery),
          ),
        ],
      ),
    ),
  );
  if (source == null) return null;
  // Downscale on the device: a full-resolution phone shot is ~8 MB, and this
  // is a thumbnail of a parcel, not an archival photo.
  return ImagePicker().pickImage(
    source: source,
    maxWidth: 1600,
    maxHeight: 1600,
    imageQuality: 85,
  );
}

/// Detail view: the photo, where it's up to, and the full update history.
class ShipmentScreen extends StatefulWidget {
  const ShipmentScreen({super.key, required this.shipmentId});
  final String shipmentId;

  @override
  State<ShipmentScreen> createState() => _ShipmentScreenState();
}

class _ShipmentScreenState extends State<ShipmentScreen> {
  final _updateCtrl = TextEditingController();
  bool _busy = false;

  @override
  void dispose() {
    _updateCtrl.dispose();
    super.dispose();
  }

  /// Run a store action with a spinner, surfacing failures as a snack bar.
  Future<void> _run(Future<void> Function() action) async {
    setState(() => _busy = true);
    try {
      await action();
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('$e')));
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<ShipmentsStore>();
    final shipment = store.byId(widget.shipmentId);
    // Deleted, or filtered out of the current list — nothing left to show.
    if (shipment == null) {
      return Scaffold(
        backgroundColor: Palette.bg,
        appBar: AppBar(title: const Text('Shipment')),
        body: const Center(
          child: Text('This shipment is no longer here.',
              style: TextStyle(color: Palette.faint)),
        ),
      );
    }

    return Scaffold(
      backgroundColor: Palette.bg,
      appBar: AppBar(
        title: Text(shipment.label, overflow: TextOverflow.ellipsis),
        actions: [
          IconButton(
            icon: const Icon(Icons.edit_outlined, color: Palette.muted),
            onPressed: () => showShipmentEditor(context, shipment: shipment),
          ),
          IconButton(
            icon: const Icon(Icons.delete_outline, color: Palette.muted),
            onPressed: () async {
              final ok = await _confirmDelete(context);
              if (!ok || !context.mounted) return;
              await context.read<ShipmentsStore>().remove(shipment);
              if (context.mounted) Navigator.of(context).pop();
            },
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 32),
        children: [
          _PhotoBlock(shipment: shipment, busy: _busy, onRun: _run),
          const SizedBox(height: 16),
          Row(
            children: [
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                decoration: BoxDecoration(
                  color: shipmentStatusColor(shipment.status).withValues(alpha: 0.14),
                  borderRadius: BorderRadius.circular(999),
                ),
                child: Text(shipment.statusLabel,
                    style: TextStyle(
                        color: shipmentStatusColor(shipment.status), fontSize: 12)),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  shipment.isLate
                      ? 'Overdue — ${shipmentEtaText(shipment.eta)}'
                      : shipmentEtaText(shipment.eta),
                  style: TextStyle(
                      color: shipment.isLate ? Palette.warn : Palette.muted,
                      fontSize: 12.5),
                ),
              ),
            ],
          ),
          if (shipment.isOpen) ...[
            const SizedBox(height: 12),
            FilledButton.icon(
              onPressed: _busy
                  ? null
                  : () => _run(() => context
                      .read<ShipmentsStore>()
                      .update(shipment, {'status': 'delivered'})),
              icon: const Icon(Icons.check, size: 18),
              label: const Text('Mark delivered'),
            ),
          ],
          if (shipment.trackingUrl != null) ...[
            const SizedBox(height: 8),
            OutlinedButton.icon(
              onPressed: () => launchUrl(Uri.parse(shipment.trackingUrl!),
                  mode: LaunchMode.externalApplication),
              icon: const Icon(Icons.open_in_new, size: 16),
              label: const Text('Track with carrier'),
            ),
          ],
          const SizedBox(height: 16),
          _details(shipment),
          if (shipment.description != null) ...[
            const SizedBox(height: 12),
            Text(shipment.description!,
                style: const TextStyle(color: Palette.fg, fontSize: 13.5, height: 1.4)),
          ],
          const SizedBox(height: 20),
          const Text('History',
              style: TextStyle(
                  color: Palette.muted, fontSize: 12, fontWeight: FontWeight.w600)),
          const SizedBox(height: 8),
          if (shipment.events.isEmpty)
            const Text('No updates yet.',
                style: TextStyle(color: Palette.faint, fontSize: 12.5))
          else
            ...shipment.events.map((e) => Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      SizedBox(
                        width: 92,
                        child: Text(
                          e.occurredAt == null
                              ? ''
                              : DateFormat('d MMM, h:mm a').format(e.occurredAt!),
                          style: const TextStyle(color: Palette.faint, fontSize: 11.5),
                        ),
                      ),
                      Expanded(
                        child: Text(e.detail,
                            style: const TextStyle(color: Palette.fg, fontSize: 12.5)),
                      ),
                    ],
                  ),
                )),
          const SizedBox(height: 12),
          TextField(
            controller: _updateCtrl,
            style: const TextStyle(color: Palette.fg, fontSize: 13.5),
            decoration: InputDecoration(
              hintText: 'Add an update…',
              isDense: true,
              suffixIcon: IconButton(
                icon: const Icon(Icons.send, size: 18, color: Palette.accent),
                onPressed: _busy
                    ? null
                    : () {
                        final text = _updateCtrl.text.trim();
                        if (text.isEmpty) return;
                        _updateCtrl.clear();
                        _run(() => context
                            .read<ShipmentsStore>()
                            .addUpdate(shipment, text));
                      },
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _details(Shipment s) {
    final rows = <(String, String)>[
      if (s.merchant != null) ('From', s.merchant!),
      if (s.carrier != null) ('Carrier', s.carrier!),
      if (s.trackingNumber != null) ('Tracking', s.trackingNumber!),
      if (s.orderRef != null) ('Order', s.orderRef!),
    ];
    if (rows.isEmpty) return const SizedBox.shrink();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: rows
          .map((r) => Padding(
                padding: const EdgeInsets.only(bottom: 4),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SizedBox(
                      width: 76,
                      child: Text(r.$1,
                          style: const TextStyle(color: Palette.faint, fontSize: 12)),
                    ),
                    Expanded(
                      child: SelectableText(r.$2,
                          style: const TextStyle(color: Palette.fg, fontSize: 12.5)),
                    ),
                  ],
                ),
              ))
          .toList(),
    );
  }
}

Future<bool> _confirmDelete(BuildContext context) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (_) => AlertDialog(
      backgroundColor: Palette.surface,
      title: const Text('Delete shipment?', style: TextStyle(color: Palette.fg)),
      content: const Text('This removes it and its history.',
          style: TextStyle(color: Palette.muted)),
      actions: [
        TextButton(
            onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
        TextButton(
          onPressed: () => Navigator.pop(context, true),
          child: const Text('Delete', style: TextStyle(color: Palette.danger)),
        ),
      ],
    ),
  );
  return ok ?? false;
}

/// The photo of what's on the way: tap to shoot or pick one, long-press-free
/// remove via the overlay button.
class _PhotoBlock extends StatelessWidget {
  const _PhotoBlock({required this.shipment, required this.busy, required this.onRun});

  final Shipment shipment;
  final bool busy;
  final Future<void> Function(Future<void> Function()) onRun;

  Future<void> _change(BuildContext context) async {
    final store = context.read<ShipmentsStore>();
    final picked = await pickShipmentPhoto(context);
    if (picked == null) return;
    final bytes = await picked.readAsBytes();
    await onRun(() => store.setPhoto(
        shipment, bytes, picked.mimeType ?? 'image/jpeg'));
  }

  @override
  Widget build(BuildContext context) {
    final store = context.read<ShipmentsStore>();
    return Stack(
      children: [
        GestureDetector(
          onTap: busy ? null : () => _change(context),
          child: Container(
            height: 180,
            width: double.infinity,
            decoration: BoxDecoration(
              color: Palette.surface,
              border: Border.all(color: Palette.raised),
              borderRadius: BorderRadius.circular(10),
            ),
            clipBehavior: Clip.antiAlias,
            child: shipment.hasPhoto
                ? Image.network(
                    store.photoUrl(shipment),
                    headers: store.photoHeaders,
                    fit: BoxFit.cover,
                    errorBuilder: (_, _, _) => const Center(
                      child: Icon(Icons.broken_image_outlined,
                          color: Palette.faint, size: 28),
                    ),
                  )
                : const Center(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(Icons.add_a_photo_outlined,
                            color: Palette.faint, size: 26),
                        SizedBox(height: 6),
                        Text('Add a photo of what\'s coming',
                            style: TextStyle(color: Palette.faint, fontSize: 12)),
                      ],
                    ),
                  ),
          ),
        ),
        if (shipment.hasPhoto)
          Positioned(
            top: 6,
            right: 6,
            child: IconButton(
              style: IconButton.styleFrom(backgroundColor: Palette.bg),
              icon: const Icon(Icons.close, size: 16, color: Palette.muted),
              onPressed:
                  busy ? null : () => onRun(() => store.removePhoto(shipment)),
            ),
          ),
      ],
    );
  }
}

/// Full-screen add/edit form. Passing [shipment] edits it; omitting it adds one.
Future<void> showShipmentEditor(BuildContext context, {Shipment? shipment}) {
  return Navigator.of(context).push(MaterialPageRoute(
    fullscreenDialog: true,
    builder: (_) => ShipmentEditorScreen(shipment: shipment),
  ));
}

class ShipmentEditorScreen extends StatefulWidget {
  const ShipmentEditorScreen({super.key, this.shipment});
  final Shipment? shipment;

  @override
  State<ShipmentEditorScreen> createState() => _ShipmentEditorScreenState();
}

class _ShipmentEditorScreenState extends State<ShipmentEditorScreen> {
  late final TextEditingController _label;
  late final TextEditingController _merchant;
  late final TextEditingController _carrier;
  late final TextEditingController _tracking;
  late final TextEditingController _trackingUrl;
  late final TextEditingController _description;
  late String _status;
  DateTime? _eta;
  bool _saving = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    final s = widget.shipment;
    _label = TextEditingController(text: s?.label ?? '');
    _merchant = TextEditingController(text: s?.merchant ?? '');
    _carrier = TextEditingController(text: s?.carrier ?? '');
    _tracking = TextEditingController(text: s?.trackingNumber ?? '');
    _trackingUrl = TextEditingController(text: s?.trackingUrl ?? '');
    _description = TextEditingController(text: s?.description ?? '');
    _status = s?.status ?? 'ordered';
    _eta = s?.eta;
  }

  @override
  void dispose() {
    for (final c in [_label, _merchant, _carrier, _tracking, _trackingUrl, _description]) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> _pickEta() async {
    final now = DateTime.now();
    final picked = await showDatePicker(
      context: context,
      initialDate: _eta ?? now,
      firstDate: now.subtract(const Duration(days: 365)),
      lastDate: now.add(const Duration(days: 730)),
    );
    if (picked == null) return;
    // Carriers promise a day, not a time. 5pm local keeps an all-day ETA from
    // reading as overdue for the whole day it's actually due.
    setState(() => _eta = DateTime(picked.year, picked.month, picked.day, 17));
  }

  /// Blank fields go up as null so the backend clears them.
  Map<String, Object?> _fields() {
    String? orNull(TextEditingController c) =>
        c.text.trim().isEmpty ? null : c.text.trim();
    return {
      'label': _label.text.trim(),
      'merchant': orNull(_merchant),
      'carrier': orNull(_carrier),
      'tracking_number': orNull(_tracking),
      'tracking_url': orNull(_trackingUrl),
      'description': orNull(_description),
      'status': _status,
      'eta': _eta?.toUtc().toIso8601String(),
    };
  }

  Future<void> _save() async {
    if (_label.text.trim().isEmpty) return;
    setState(() {
      _saving = true;
      _error = null;
    });
    final store = context.read<ShipmentsStore>();
    try {
      final existing = widget.shipment;
      if (existing == null) {
        await store.create(_fields());
      } else {
        await store.update(existing, _fields());
      }
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) {
        setState(() {
          _error = '$e';
          _saving = false;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Palette.bg,
      appBar: AppBar(
        title: Text(widget.shipment == null ? 'Track a delivery' : 'Edit shipment'),
        actions: [
          TextButton(
            onPressed: _saving ? null : _save,
            child: const Text('Save'),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 32),
        children: [
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(_error!,
                  style: const TextStyle(color: Palette.danger, fontSize: 12.5)),
            ),
          _field(_label, "What's on the way?", autofocus: widget.shipment == null),
          _field(_merchant, 'From (shop)'),
          _field(_carrier, 'Carrier'),
          _field(_tracking, 'Tracking number'),
          _field(_trackingUrl, 'Tracking link'),
          const SizedBox(height: 4),
          DropdownButtonFormField<String>(
            initialValue: _status,
            dropdownColor: Palette.surface,
            style: const TextStyle(color: Palette.fg, fontSize: 14),
            decoration: const InputDecoration(labelText: 'Status', isDense: true),
            items: shipmentStatuses.entries
                .map((e) => DropdownMenuItem(value: e.key, child: Text(e.value)))
                .toList(),
            onChanged: (v) => setState(() => _status = v ?? _status),
          ),
          const SizedBox(height: 12),
          ListTile(
            contentPadding: EdgeInsets.zero,
            title: Text(
              _eta == null
                  ? 'No expected date'
                  : 'Expected ${DateFormat('EEE d MMM yyyy').format(_eta!)}',
              style: const TextStyle(color: Palette.fg, fontSize: 14),
            ),
            trailing: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (_eta != null)
                  IconButton(
                    icon: const Icon(Icons.close, size: 18, color: Palette.faint),
                    onPressed: () => setState(() => _eta = null),
                  ),
                IconButton(
                  icon: const Icon(Icons.event, size: 20, color: Palette.muted),
                  onPressed: _pickEta,
                ),
              ],
            ),
          ),
          _field(_description, 'Notes', maxLines: 3),
        ],
      ),
    );
  }

  Widget _field(TextEditingController c, String label,
      {int maxLines = 1, bool autofocus = false}) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: TextField(
        controller: c,
        autofocus: autofocus,
        maxLines: maxLines,
        style: const TextStyle(color: Palette.fg, fontSize: 14),
        decoration: InputDecoration(labelText: label, isDense: true),
      ),
    );
  }
}
